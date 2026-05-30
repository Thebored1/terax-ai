use std::collections::{hash_map::DefaultHasher, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::modules::fs::to_canon;
use crate::modules::git::process::{ensure_success, git_stdout_line_opt, run_git};
use crate::modules::git::utils::{canonical_dir, is_safe_pathspec, resolve_within_repo};
use crate::modules::workspace::{WorkspaceEnv, WorkspaceRegistry};

const MANIFEST: &str = "manifest.json";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMeta {
    pub id: String,
    pub session_id: String,
    pub workspace_root: String,
    pub created_at: u64,
    pub prompt_preview: String,
    pub file_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRestoreResult {
    pub restored: usize,
    pub deleted: usize,
    pub skipped: usize,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotManifest {
    meta: SnapshotMeta,
    files: Vec<SnapshotFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotFile {
    path: String,
    state: FileState,
    blob: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FileState {
    Present,
    Missing,
    Skipped,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotRestoredEvent {
    snapshot_id: String,
    paths: Vec<String>,
}

#[tauri::command]
pub fn snapshot_create(
    app: tauri::AppHandle,
    registry: tauri::State<'_, WorkspaceRegistry>,
    workspace: Option<WorkspaceEnv>,
    workspace_root: String,
    session_id: String,
    prompt_preview: String,
) -> Result<SnapshotMeta, String> {
    let workspace = WorkspaceEnv::from_option(workspace);
    let store = snapshots_store_dir(&app)?;
    create_snapshot(
        &registry,
        &workspace,
        &store,
        &workspace_root,
        &session_id,
        &prompt_preview,
    )
}

#[tauri::command]
pub fn snapshot_list(
    app: tauri::AppHandle,
    registry: tauri::State<'_, WorkspaceRegistry>,
    workspace: Option<WorkspaceEnv>,
    workspace_root: String,
    session_id: String,
) -> Result<Vec<SnapshotMeta>, String> {
    let workspace = WorkspaceEnv::from_option(workspace);
    let root = resolve_workspace_local_root(&registry, &workspace, &workspace_root)?;
    let dir = workspace_session_dir(&snapshots_store_dir(&app)?, Path::new(&root), &session_id);
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        let manifest_path = entry.path().join(MANIFEST);
        let Ok(manifest) = read_manifest(&manifest_path) else {
            continue;
        };
        out.push(manifest.meta);
    }
    out.sort_by_key(|m| m.created_at);
    Ok(out)
}

#[tauri::command]
pub fn snapshot_restore(
    app: tauri::AppHandle,
    registry: tauri::State<'_, WorkspaceRegistry>,
    workspace: Option<WorkspaceEnv>,
    workspace_root: String,
    snapshot_id: String,
) -> Result<SnapshotRestoreResult, String> {
    let workspace = WorkspaceEnv::from_option(workspace);
    let store = snapshots_store_dir(&app)?;
    let result = restore_snapshot(&registry, &workspace, &store, &workspace_root, &snapshot_id)?;
    let _ = app.emit(
        "snapshot:restored",
        SnapshotRestoredEvent {
            snapshot_id,
            paths: result.paths.clone(),
        },
    );
    Ok(result)
}

#[tauri::command]
pub fn snapshot_delete(
    app: tauri::AppHandle,
    registry: tauri::State<'_, WorkspaceRegistry>,
    workspace: Option<WorkspaceEnv>,
    workspace_root: String,
    snapshot_id: String,
) -> Result<(), String> {
    let workspace = WorkspaceEnv::from_option(workspace);
    let root = resolve_workspace_local_root(&registry, &workspace, &workspace_root)?;
    let workspace_dir = workspace_dir(&snapshots_store_dir(&app)?, Path::new(&root));
    let Ok(session_dirs) = fs::read_dir(workspace_dir) else {
        return Ok(());
    };
    for session_dir in session_dirs.flatten() {
        let candidate = session_dir.path().join(&snapshot_id);
        if candidate.is_dir() {
            fs::remove_dir_all(candidate).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Ok(())
}

fn create_snapshot(
    registry: &WorkspaceRegistry,
    workspace: &WorkspaceEnv,
    store: &Path,
    workspace_root: &str,
    session_id: &str,
    prompt_preview: &str,
) -> Result<SnapshotMeta, String> {
    if let Ok(repo) = repo_root_from_cwd(registry, workspace, workspace_root) {
        return create_snapshot_for_repo(
            registry,
            workspace,
            store,
            &repo.git_path,
            session_id,
            prompt_preview,
        );
    }
    Err("snapshots require a Git repository".into())
}

fn create_snapshot_for_repo(
    registry: &WorkspaceRegistry,
    workspace: &WorkspaceEnv,
    store: &Path,
    repo_root: &str,
    session_id: &str,
    prompt_preview: &str,
) -> Result<SnapshotMeta, String> {
    let repo = canonical_dir(registry, repo_root, workspace).map_err(|e| e.to_string())?;
    let rels = git_visible_files(workspace, &repo.git_path)?;
    let created_at = now_ms();
    let id = format!("snap-{created_at}-{}", rels.len());
    let snapshot_dir = workspace_session_dir(store, &repo.local_path, session_id).join(&id);
    let files_dir = snapshot_dir.join("files");
    fs::create_dir_all(&files_dir).map_err(|e| e.to_string())?;

    let mut files = Vec::with_capacity(rels.len());
    for (idx, rel) in rels.iter().enumerate() {
        let target = resolve_within_repo(&repo.local_path, rel).map_err(|e| e.to_string())?;
        let meta = match fs::symlink_metadata(&target) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                files.push(SnapshotFile {
                    path: rel.clone(),
                    state: FileState::Missing,
                    blob: None,
                });
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            files.push(SnapshotFile {
                path: rel.clone(),
                state: FileState::Skipped,
                blob: None,
            });
            continue;
        }
        let blob = format!("{idx}.bin");
        fs::copy(&target, files_dir.join(&blob)).map_err(|e| e.to_string())?;
        files.push(SnapshotFile {
            path: rel.clone(),
            state: FileState::Present,
            blob: Some(blob),
        });
    }

    let meta = SnapshotMeta {
        id,
        session_id: session_id.to_string(),
        workspace_root: to_canon(&repo.local_path),
        created_at,
        prompt_preview: prompt_preview.chars().take(160).collect(),
        file_count: files.len(),
    };
    let manifest = SnapshotManifest {
        meta: meta.clone(),
        files,
    };
    write_manifest(&snapshot_dir.join(MANIFEST), &manifest)?;
    Ok(meta)
}

fn restore_snapshot(
    registry: &WorkspaceRegistry,
    workspace: &WorkspaceEnv,
    store: &Path,
    workspace_root: &str,
    snapshot_id: &str,
) -> Result<SnapshotRestoreResult, String> {
    if let Ok(repo) = repo_root_from_cwd(registry, workspace, workspace_root) {
        return restore_snapshot_for_repo(
            workspace,
            store,
            &to_canon(&repo.local_path),
            &repo.git_path,
            snapshot_id,
        );
    }
    Err("snapshots require a Git repository".into())
}

fn resolve_workspace_local_root(
    registry: &WorkspaceRegistry,
    workspace: &WorkspaceEnv,
    workspace_root: &str,
) -> Result<String, String> {
    if let Ok(repo) = repo_root_from_cwd(registry, workspace, workspace_root) {
        return Ok(to_canon(&repo.local_path));
    }
    let root = canonical_dir(registry, workspace_root, workspace).map_err(|e| e.to_string())?;
    if !registry.is_authorized(&root.local_path) {
        return Err("workspace is not authorized".into());
    }
    Ok(to_canon(&root.local_path))
}


fn restore_snapshot_for_repo(
    workspace: &WorkspaceEnv,
    store: &Path,
    repo_local_root: &str,
    repo_git_root: &str,
    snapshot_id: &str,
) -> Result<SnapshotRestoreResult, String> {
    let repo_local = fs::canonicalize(repo_local_root).map_err(|e| e.to_string())?;
    let snapshot_dir = find_snapshot_dir(store, &repo_local, snapshot_id)?;
    let manifest = read_manifest(&snapshot_dir.join(MANIFEST))?;
    if manifest.meta.workspace_root != to_canon(&repo_local) {
        return Err("snapshot belongs to a different workspace".into());
    }

    let snapshot_paths: HashSet<String> = manifest.files.iter().map(|f| f.path.clone()).collect();
    let current_paths = git_visible_files(workspace, repo_git_root)?;
    let mut restored = 0;
    let mut deleted = 0;
    let mut skipped = 0;
    let mut changed = Vec::new();

    for rel in current_paths {
        if snapshot_paths.contains(&rel) {
            continue;
        }
        let target = snapshot_rel_to_path(&repo_local, &rel)?;
        if target.is_file() || target.is_symlink() {
            fs::remove_file(&target).map_err(|e| e.to_string())?;
            deleted += 1;
            changed.push(to_canon(&target));
        } else if target.is_dir() {
            fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
            deleted += 1;
            changed.push(to_canon(&target));
        }
    }

    for file in manifest.files {
        let target = snapshot_rel_to_path(&repo_local, &file.path)?;
        match file.state {
            FileState::Present => {
                let Some(blob) = file.blob else {
                    skipped += 1;
                    continue;
                };
                let source = snapshot_dir.join("files").join(blob);
                if target.is_file() && files_equal(&source, &target).map_err(|e| e.to_string())? {
                    continue;
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::copy(source, &target).map_err(|e| e.to_string())?;
                restored += 1;
                changed.push(to_canon(&target));
            }
            FileState::Missing => {
                if target.exists() || target.is_symlink() {
                    fs::remove_file(&target).map_err(|e| e.to_string())?;
                    deleted += 1;
                    changed.push(to_canon(&target));
                }
            }
            FileState::Skipped => {
                skipped += 1;
            }
        }
    }

    Ok(SnapshotRestoreResult {
        restored,
        deleted,
        skipped,
        paths: changed,
    })
}

fn files_equal(a: &Path, b: &Path) -> std::io::Result<bool> {
    let am = fs::metadata(a)?;
    let bm = fs::metadata(b)?;
    if am.len() != bm.len() {
        return Ok(false);
    }
    let mut ar = BufReader::new(fs::File::open(a)?);
    let mut br = BufReader::new(fs::File::open(b)?);
    let mut ab = [0_u8; 8192];
    let mut bb = [0_u8; 8192];
    loop {
        let an = ar.read(&mut ab)?;
        let bn = br.read(&mut bb)?;
        if an != bn {
            return Ok(false);
        }
        if an == 0 {
            return Ok(true);
        }
        if ab[..an] != bb[..bn] {
            return Ok(false);
        }
    }
}

fn snapshot_rel_to_path(repo_root: &Path, rel: &str) -> Result<PathBuf, String> {
    if !is_safe_pathspec(rel) || Path::new(rel).is_absolute() {
        return Err(format!("invalid path: {rel}"));
    }
    for component in Path::new(rel).components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(format!("invalid path: {rel}"));
        }
    }
    let joined = repo_root.join(rel);
    if let Ok(canonical) = fs::canonicalize(&joined) {
        if !canonical.starts_with(repo_root) {
            return Err(format!("path outside workspace: {}", canonical.display()));
        }
        return Ok(canonical);
    }
    if let Some(parent) = joined.parent() {
        if parent.exists() {
            let canonical_parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
            if !canonical_parent.starts_with(repo_root) {
                return Err(format!(
                    "path outside workspace: {}",
                    canonical_parent.display()
                ));
            }
        }
    }
    Ok(joined)
}

fn repo_root_from_cwd(
    registry: &WorkspaceRegistry,
    workspace: &WorkspaceEnv,
    cwd: &str,
) -> Result<crate::modules::git::utils::ResolvedGitDirectory, String> {
    if let Ok(repo) = repo_root_from_single_cwd(registry, workspace, cwd) {
        return Ok(repo);
    }
    if let Some(launch) = crate::modules::workspace::launch_cwd_snapshot() {
        let launch = to_canon(&launch);
        if launch != cwd {
            if let Ok(repo) = repo_root_from_single_cwd(registry, workspace, &launch) {
                return Ok(repo);
            }
        }
    }
    repo_root_from_single_cwd(registry, workspace, cwd)
}

fn repo_root_from_single_cwd(
    registry: &WorkspaceRegistry,
    workspace: &WorkspaceEnv,
    cwd: &str,
) -> Result<crate::modules::git::utils::ResolvedGitDirectory, String> {
    let cwd = canonical_dir(registry, &cwd, workspace).map_err(|e| e.to_string())?;
    if !registry.is_authorized(&cwd.local_path) {
        return Err("workspace is not authorized".into());
    }
    let root = git_stdout_line_opt(workspace, &cwd.git_path, ["rev-parse", "--show-toplevel"])
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "snapshots require a Git repository".to_string())?;
    let repo = canonical_dir(registry, &root, workspace).map_err(|e| e.to_string())?;
    let _ = registry.authorize(&repo.local_path);
    Ok(repo)
}

fn git_visible_files(workspace: &WorkspaceEnv, repo_root: &str) -> Result<Vec<String>, String> {
    let output = run_git(
        workspace,
        Some(repo_root),
        [
            OsStr::new("ls-files"),
            OsStr::new("-z"),
            OsStr::new("--cached"),
            OsStr::new("--others"),
            OsStr::new("--exclude-standard"),
        ],
        30,
    )
    .map_err(|e| e.to_string())?;
    ensure_success(&output, "git ls-files failed").map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for raw in output.stdout.split(|b| *b == 0) {
        if raw.is_empty() {
            continue;
        }
        let rel = String::from_utf8_lossy(raw).replace('\\', "/");
        if rel == ".git" || rel.starts_with(".git/") || !is_safe_pathspec(&rel) {
            continue;
        }
        out.push(rel);
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn snapshots_store_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("snapshots");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn workspace_dir(store: &Path, repo_root: &Path) -> PathBuf {
    store.join(format!("{:x}", hash_string(&to_canon(repo_root))))
}

fn workspace_session_dir(store: &Path, repo_root: &Path, session_id: &str) -> PathBuf {
    workspace_dir(store, repo_root).join(safe_name(session_id))
}

fn find_snapshot_dir(store: &Path, repo_root: &Path, snapshot_id: &str) -> Result<PathBuf, String> {
    let workspace_dir = workspace_dir(store, repo_root);
    let session_dirs = fs::read_dir(workspace_dir).map_err(|e| e.to_string())?;
    for session_dir in session_dirs.flatten() {
        let candidate = session_dir.path().join(snapshot_id);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    Err("snapshot not found".into())
}

fn write_manifest(path: &Path, manifest: &SnapshotManifest) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    fs::write(path, bytes).map_err(|e| e.to_string())
}

fn read_manifest(path: &Path) -> Result<SnapshotManifest, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn hash_string(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    struct Repo {
        dir: TempDir,
        store: TempDir,
        registry: WorkspaceRegistry,
    }

    impl Repo {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            run(&dir, ["init"]);
            run(&dir, ["config", "user.email", "test@example.com"]);
            run(&dir, ["config", "user.name", "Test"]);
            let registry = WorkspaceRegistry::default();
            registry.authorize(dir.path()).unwrap();
            Self {
                dir,
                store: tempfile::tempdir().unwrap(),
                registry,
            }
        }

        fn path(&self) -> String {
            to_canon(self.dir.path())
        }

        fn write(&self, rel: &str, content: &str) {
            let path = self.dir.path().join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }

        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.dir.path().join(rel)).unwrap()
        }
    }

    fn run<I, S>(dir: &TempDir, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let status = Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn snapshot(repo: &Repo) -> SnapshotMeta {
        create_snapshot_for_repo(
            &repo.registry,
            &WorkspaceEnv::Local,
            repo.store.path(),
            &repo.path(),
            "s1",
            "prompt",
        )
        .unwrap()
    }

    fn restore(repo: &Repo, id: &str) {
        restore_snapshot_for_repo(
            &WorkspaceEnv::Local,
            repo.store.path(),
            &repo.path(),
            &repo.path(),
            id,
        )
        .unwrap();
    }

    #[test]
    fn restores_modified_file() {
        let repo = Repo::new();
        repo.write("a.txt", "before");
        run(&repo.dir, ["add", "."]);
        run(&repo.dir, ["commit", "-m", "init"]);
        let s = snapshot(&repo);
        repo.write("a.txt", "after");
        restore(&repo, &s.id);
        assert_eq!(repo.read("a.txt"), "before");
    }

    #[test]
    fn restores_deleted_file() {
        let repo = Repo::new();
        repo.write("a.txt", "before");
        run(&repo.dir, ["add", "."]);
        run(&repo.dir, ["commit", "-m", "init"]);
        let s = snapshot(&repo);
        fs::remove_file(repo.dir.path().join("a.txt")).unwrap();
        restore(&repo, &s.id);
        assert_eq!(repo.read("a.txt"), "before");
    }

    #[test]
    fn restores_untracked_file_content() {
        let repo = Repo::new();
        repo.write("a.txt", "dirty");
        let s = snapshot(&repo);
        repo.write("a.txt", "changed");
        restore(&repo, &s.id);
        assert_eq!(repo.read("a.txt"), "dirty");
    }

    #[test]
    fn deletes_file_created_after_checkpoint() {
        let repo = Repo::new();
        repo.write("a.txt", "before");
        run(&repo.dir, ["add", "."]);
        run(&repo.dir, ["commit", "-m", "init"]);
        let s = snapshot(&repo);
        repo.write("new.txt", "new");
        restore(&repo, &s.id);
        assert!(!repo.dir.path().join("new.txt").exists());
    }

    #[test]
    fn excludes_ignored_files() {
        let repo = Repo::new();
        repo.write(".gitignore", "ignored/\n");
        repo.write("a.txt", "before");
        run(&repo.dir, ["add", "."]);
        run(&repo.dir, ["commit", "-m", "init"]);
        fs::create_dir_all(repo.dir.path().join("ignored")).unwrap();
        fs::write(repo.dir.path().join("ignored/cache.txt"), "cache").unwrap();
        let s = snapshot(&repo);
        let manifest = read_manifest(
            &repo
                .store
                .path()
                .join(format!("{:x}", hash_string(&repo.path())))
                .join("s1")
                .join(&s.id)
                .join(MANIFEST),
        )
        .unwrap();
        assert!(!manifest
            .files
            .iter()
            .any(|f| f.path.starts_with("ignored/")));
    }

    #[test]
    fn rejects_unsafe_path_on_restore() {
        let repo = Repo::new();
        repo.write("a.txt", "before");
        let s = snapshot(&repo);
        let manifest_path = repo
            .store
            .path()
            .join(format!("{:x}", hash_string(&repo.path())))
            .join("s1")
            .join(&s.id)
            .join(MANIFEST);
        let mut manifest = read_manifest(&manifest_path).unwrap();
        manifest.files.push(SnapshotFile {
            path: "../evil.txt".into(),
            state: FileState::Missing,
            blob: None,
        });
        write_manifest(&manifest_path, &manifest).unwrap();
        let err = restore_snapshot_for_repo(
            &WorkspaceEnv::Local,
            repo.store.path(),
            &repo.path(),
            &repo.path(),
            &s.id,
        )
        .unwrap_err();
        assert!(err.contains("invalid path") || err.contains("outside"));
    }

    #[test]
    fn restore_skips_unchanged_files() {
        let repo = Repo::new();
        repo.write("a.txt", "before");
        run(&repo.dir, ["add", "."]);
        run(&repo.dir, ["commit", "-m", "init"]);
        let s = snapshot(&repo);
        let result = restore_snapshot_for_repo(
            &WorkspaceEnv::Local,
            repo.store.path(),
            &repo.path(),
            &repo.path(),
            &s.id,
        )
        .unwrap();
        assert_eq!(result.restored, 0);
        assert_eq!(result.deleted, 0);
        assert!(result.paths.is_empty());
    }
}
