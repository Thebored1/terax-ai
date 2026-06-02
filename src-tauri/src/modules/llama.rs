use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use shared_child::SharedChild;

#[derive(Default)]
pub struct LlamaRuntimeState {
    inner: RwLock<Option<LlamaRuntimeProc>>,
}

struct LlamaRuntimeProc {
    child: Arc<SharedChild>,
    binary: String,
    source: String,
    model_id: String,
    base_url: String,
    command_line: String,
    started_at_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct LlamaRuntimeStartInput {
    pub base_url: String,
    pub model_id: String,
    pub source: String,
    pub binary_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LlamaRuntimeStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub binary: Option<String>,
    pub source: Option<String>,
    pub model_id: Option<String>,
    pub base_url: Option<String>,
    pub command_line: Option<String>,
    pub started_at_ms: Option<u64>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn parse_host_port(base_url: &str) -> Result<(String, u16), String> {
    let parsed = reqwest::Url::parse(base_url.trim()).map_err(|e| e.to_string())?;
    match parsed.scheme() {
        "http" | "https" => {}
        s => return Err(format!("unsupported base url scheme: {s}")),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "base url is missing host".to_string())?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "base url is missing port".to_string())?;
    Ok((host.to_string(), port))
}

fn resolve_binary(binary_path: Option<String>) -> String {
    if let Some(p) = binary_path {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    #[cfg(windows)]
    {
        "llama.exe".to_string()
    }
    #[cfg(not(windows))]
    {
        "llama".to_string()
    }
}

fn resolve_source_arg(source: &str) -> Result<(&'static str, String), String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("source is empty; set a Hugging Face repo or GGUF path".into());
    }
    let looks_like_file = trimmed.to_ascii_lowercase().ends_with(".gguf")
        || trimmed.contains('\\')
        || trimmed.starts_with('/')
        || trimmed.contains(":\\");
    if Path::new(trimmed).exists() {
        return Ok(("--model", trimmed.to_string()));
    }
    if looks_like_file {
        return Err(format!("GGUF path not found: {trimmed}"));
    }
    Ok(("--hf-repo", trimmed.to_string()))
}

fn kill_and_wait(proc: &LlamaRuntimeProc) {
    let _ = proc.child.kill();
    let _ = proc.child.wait();
}

#[tauri::command]
pub fn llama_runtime_start(
    state: tauri::State<'_, LlamaRuntimeState>,
    input: LlamaRuntimeStartInput,
) -> Result<LlamaRuntimeStatus, String> {
    let base_url = input.base_url.trim().to_string();
    let model_id = input.model_id.trim().to_string();
    if base_url.is_empty() {
        return Err("base url is empty".into());
    }
    if model_id.is_empty() {
        return Err("model id is empty".into());
    }
    let (host, port) = parse_host_port(&base_url)?;
    let (source_flag, source_value) = resolve_source_arg(&input.source)?;
    let binary = resolve_binary(input.binary_path);

    let mut guard = state.inner.write().unwrap();
    if let Some(current) = guard.as_ref() {
        match current.child.try_wait() {
            Ok(None) => return Err("llama runtime is already running".into()),
            Ok(Some(_)) | Err(_) => {
                *guard = None;
            }
        }
    }

    let mut cmd = Command::new(&binary);
    cmd.arg("serve")
        .arg(source_flag)
        .arg(&source_value)
        .arg("--alias")
        .arg(&model_id)
        .arg("--host")
        .arg(&host)
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::modules::proc::hide_console(&mut cmd);

    let command_line = format!(
        "{binary} serve {source_flag} {source_value} --alias {model_id} --host {host} --port {port}"
    );
    let child = Arc::new(SharedChild::spawn(&mut cmd).map_err(|e| e.to_string())?);

    *guard = Some(LlamaRuntimeProc {
        child: Arc::clone(&child),
        binary: binary.clone(),
        source: source_value.clone(),
        model_id: model_id.clone(),
        base_url: base_url.clone(),
        command_line: command_line.clone(),
        started_at_ms: now_ms(),
    });

    Ok(LlamaRuntimeStatus {
        running: true,
        pid: Some(child.id()),
        binary: Some(binary),
        source: Some(source_value),
        model_id: Some(model_id),
        base_url: Some(base_url),
        command_line: Some(command_line),
        started_at_ms: guard.as_ref().map(|p| p.started_at_ms),
    })
}

#[tauri::command]
pub fn llama_runtime_stop(state: tauri::State<'_, LlamaRuntimeState>) -> Result<(), String> {
    let mut guard = state.inner.write().unwrap();
    if let Some(proc) = guard.as_ref() {
        kill_and_wait(proc);
    }
    *guard = None;
    Ok(())
}

#[tauri::command]
pub fn llama_runtime_status(
    state: tauri::State<'_, LlamaRuntimeState>,
) -> Result<LlamaRuntimeStatus, String> {
    let mut guard = state.inner.write().unwrap();
    if let Some(proc) = guard.as_ref() {
        match proc.child.try_wait() {
            Ok(None) => {
                return Ok(LlamaRuntimeStatus {
                    running: true,
                    pid: Some(proc.child.id()),
                    binary: Some(proc.binary.clone()),
                    source: Some(proc.source.clone()),
                    model_id: Some(proc.model_id.clone()),
                    base_url: Some(proc.base_url.clone()),
                    command_line: Some(proc.command_line.clone()),
                    started_at_ms: Some(proc.started_at_ms),
                });
            }
            Ok(Some(_)) | Err(_) => {
                *guard = None;
            }
        }
    }
    Ok(LlamaRuntimeStatus {
        running: false,
        pid: None,
        binary: None,
        source: None,
        model_id: None,
        base_url: None,
        command_line: None,
        started_at_ms: None,
    })
}

