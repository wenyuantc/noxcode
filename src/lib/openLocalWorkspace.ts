import { open } from "@tauri-apps/plugin-dialog";

import { useWorkspaceStore } from "@/stores/workspaceStore";

export async function openLocalWorkspace(): Promise<boolean> {
  const selected = await open({ directory: true, multiple: false });
  if (typeof selected !== "string") return false;
  const name = selected.split(/[/\\]/).filter(Boolean).pop() ?? selected;
  await useWorkspaceStore.getState().create({
    name,
    workspace_type: "local",
    repo_path: selected,
  });
  return true;
}
