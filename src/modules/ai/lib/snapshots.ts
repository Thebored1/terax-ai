import { invoke } from "@tauri-apps/api/core";
import { currentWorkspaceEnv } from "@/modules/workspace";

export type SnapshotMeta = {
  id: string;
  sessionId: string;
  workspaceRoot: string;
  createdAt: number;
  promptPreview: string;
  fileCount: number;
};

export type SnapshotRestoreResult = {
  restored: number;
  deleted: number;
  skipped: number;
  paths: string[];
};

export async function createSnapshot(
  sessionId: string,
  promptPreview: string,
  workspaceRoot: string,
): Promise<SnapshotMeta> {
  return invoke<SnapshotMeta>("snapshot_create", {
    workspace: currentWorkspaceEnv(),
    workspaceRoot,
    sessionId,
    promptPreview,
  });
}

export async function listSnapshots(
  sessionId: string,
  workspaceRoot: string,
): Promise<SnapshotMeta[]> {
  return invoke<SnapshotMeta[]>("snapshot_list", {
    workspace: currentWorkspaceEnv(),
    workspaceRoot,
    sessionId,
  });
}

export async function restoreSnapshot(
  snapshotId: string,
  workspaceRoot: string,
): Promise<SnapshotRestoreResult> {
  return invoke<SnapshotRestoreResult>("snapshot_restore", {
    workspace: currentWorkspaceEnv(),
    workspaceRoot,
    snapshotId,
  });
}

export async function deleteSnapshot(
  snapshotId: string,
  workspaceRoot: string,
): Promise<void> {
  return invoke<void>("snapshot_delete", {
    workspace: currentWorkspaceEnv(),
    workspaceRoot,
    snapshotId,
  });
}
