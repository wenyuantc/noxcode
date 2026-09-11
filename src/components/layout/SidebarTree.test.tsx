import { I18nextProvider } from "react-i18next";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import i18n from "@/lib/i18n";
import { SidebarTree } from "@/components/layout/SidebarTree";
import type { Workspace } from "@/lib/types";

vi.mock("@/lib/backend", () => ({
  deleteWorkspace: vi.fn(),
  ensureScratchWorkspace: vi.fn(),
  listAgentSessions: vi.fn(),
  listWorkspaces: vi.fn(),
  renameAgentSession: vi.fn(),
  setAgentSessionArchived: vi.fn(),
  setAgentSessionPinned: vi.fn(),
  updateWorkspace: vi.fn(),
}));

// Server rendering uses Zustand's initial snapshot; these views need the current test state.
vi.mock("@/stores/workspaceStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/workspaceStore")>();
  return {
    useWorkspaceStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useWorkspaceStore.getState>) => unknown) =>
        selector(actual.useWorkspaceStore.getState()),
      actual.useWorkspaceStore,
    ),
  };
});

vi.mock("@/stores/sessionStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/sessionStore")>();
  return {
    useSessionStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useSessionStore.getState>) => unknown) =>
        selector(actual.useSessionStore.getState()),
      actual.useSessionStore,
    ),
  };
});

import { useWorkspaceStore } from "@/stores/workspaceStore";

function sampleWorkspace(id: string, name: string): Workspace {
  return {
    id,
    name,
    workspace_type: "local",
    repo_path: `/repos/${id}`,
    ssh_config_id: null,
    remote_repo_path: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

function renderSidebar(): string {
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <SidebarTree />
    </I18nextProvider>,
  );
}

function rowTags(html: string): string[] {
  return html.match(/<div[^>]*data-workspace-row="[^"]*"[^>]*>/g) ?? [];
}

describe("SidebarTree workspace reorder", () => {
  it("marks every workspace header as a drag handle", () => {
    useWorkspaceStore.setState({
      workspaces: [
        sampleWorkspace("ws-1", "Alpha"),
        sampleWorkspace("ws-2", "Beta"),
        sampleWorkspace("ws-3", "Gamma"),
      ],
      sessions: [],
    });

    const html = renderSidebar();
    const rows = rowTags(html);

    expect(rows).toHaveLength(3);
    expect(rows[0]).toContain('data-workspace-row="ws-1"');
    expect(rows[2]).toContain('data-workspace-row="ws-3"');
  });

  it("shows a grab cursor while idle and keeps the row clickable", () => {
    useWorkspaceStore.setState({
      workspaces: [sampleWorkspace("ws-1", "Alpha")],
      sessions: [],
    });

    const html = renderSidebar();
    const rows = rowTags(html);

    expect(rows).toHaveLength(1);
    expect(rows[0]).toContain("cursor-grab");
    expect(rows[0]).not.toContain("cursor-grabbing");
    // The expand / activate button is rendered next to the drag-suppressed action area.
    expect(html).toContain('class="flex min-w-0 flex-1 items-center gap-1.5 rounded-md text-left');
    expect(html).toContain("data-no-drag");
  });

  it("renders no drop indicator while idle", () => {
    useWorkspaceStore.setState({
      workspaces: [sampleWorkspace("ws-1", "Alpha"), sampleWorkspace("ws-2", "Beta")],
      sessions: [],
    });

    const html = renderSidebar();

    expect(html).not.toContain("pointer-events-none absolute inset-x-2 h-0.5 rounded-full");
  });

  it("keeps the keyboard-reachable move entries in the workspace menu", () => {
    useWorkspaceStore.setState({
      workspaces: [sampleWorkspace("ws-1", "Alpha"), sampleWorkspace("ws-2", "Beta")],
      sessions: [],
    });

    const html = renderSidebar();

    // The menu items live in a portal, so only the trigger is part of the static markup.
    expect(html).toContain('aria-label="工作区操作"');
  });
});
