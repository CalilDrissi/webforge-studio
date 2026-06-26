import { invoke } from "@tauri-apps/api/core";
import Database from "@tauri-apps/plugin-sql";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { initThemeBuilder } from "./theme-builder";

interface Workspace {
  id: number;
  name: string;
  slug: string;
  kind: string; // "local-folder" | "local-import" | "remote"
  folder_path: string;
  start_page: string;
  server_url: string | null;
  created_at: number;
  last_opened_at: number | null;
}

let db: Database | null = null;
const sitesListEl = document.querySelector<HTMLUListElement>("#sites-list")!;
const emptyEl = document.querySelector<HTMLElement>("#sites-empty")!;
const addFormEl = document.querySelector<HTMLElement>("#add-form")!;
const nameInputEl = document.querySelector<HTMLInputElement>("#site-name")!;
const urlInputEl = document.querySelector<HTMLInputElement>("#site-url")!;
const statusEl = document.querySelector<HTMLElement>("#add-status")!;
const kindSelectEl = document.querySelector<HTMLSelectElement>("#site-kind")!;
const localRowEl = document.querySelector<HTMLElement>("#site-local-row")!;
const remoteRowEl = document.querySelector<HTMLElement>("#site-remote-row")!;
const folderPathEl = document.querySelector<HTMLInputElement>("#site-folder-path")!;
const pickFolderBtn = document.querySelector<HTMLElement>("#site-pick-folder")!;
const startPageEl = document.querySelector<HTMLInputElement>("#site-start-page")!;
const folderLabelEl = document.querySelector<HTMLElement>("#site-folder-label")!;
const serverStatusEl = document.querySelector<HTMLElement>("#server-status")!;
const workspacesRootLabelEl = document.querySelector<HTMLElement>("#workspaces-root-label")!;

async function loadDb(): Promise<Database> {
  if (db) return db;
  db = await Database.load("sqlite:webforge.db");
  return db;
}

async function loadWorkspaces(): Promise<void> {
  const database = await loadDb();
  const rows = await database.select<Workspace[]>(
    "SELECT id, name, slug, kind, folder_path, start_page, server_url, created_at, last_opened_at FROM workspaces ORDER BY last_opened_at DESC, created_at DESC;"
  );
  await renderWorkspaces(rows);
}

async function renderWorkspaces(workspaces: Workspace[]): Promise<void> {
  sitesListEl.innerHTML = "";
  if (workspaces.length === 0) {
    emptyEl.classList.remove("hidden");
    return;
  }
  emptyEl.classList.add("hidden");

  // query open-state for all workspaces in parallel
  const openStates = await Promise.all(
    workspaces.map((w) => invoke<boolean>("is_workspace_open", { workspaceId: w.id }).catch(() => false))
  );

  for (let i = 0; i < workspaces.length; i++) {
    const ws = workspaces[i];
    const isOpen = openStates[i];
    const li = document.createElement("li");
    li.className = "site-card" + (isOpen ? " running" : "");
    const kindBadge = kindLabel(ws.kind);
    const locationLabel = ws.kind === "remote" ? ws.server_url : ws.folder_path;
    const statusBadge = isOpen
      ? `<span class="status-badge running">Running</span>`
      : `<span class="status-badge stopped">Stopped</span>`;
    li.innerHTML = `
      <div class="site-info">
        <div class="site-name-row">
          <span class="site-name"></span>
          ${statusBadge}
        </div>
        <div class="site-url"></div>
        <div class="site-meta"><span class="badge-mini kind-${ws.kind.replace(/_/g, "-")}"></span></div>
      </div>
      <div class="site-actions">
        ${isOpen ? `<button class="btn stop-btn">Stop</button>` : `<button class="btn primary open-btn">Open</button>`}
        <button class="btn danger delete-btn">Delete</button>
      </div>
    `;
    li.querySelector<HTMLElement>(".site-name")!.textContent = ws.name;
    li.querySelector<HTMLElement>(".site-url")!.textContent = locationLabel || ws.slug;
    li.querySelector<HTMLElement>(".kind-" + ws.kind.replace(/_/g, "-"))!.textContent = kindBadge;
    const meta = li.querySelector<HTMLElement>(".site-meta")!;
    const time = ws.last_opened_at
      ? `Last opened ${new Date(ws.last_opened_at * 1000).toLocaleString()}`
      : `Added ${new Date(ws.created_at * 1000).toLocaleDateString()}`;
    meta.appendChild(document.createTextNode(" " + time));

    if (isOpen) {
      li.querySelector<HTMLElement>(".stop-btn")!.addEventListener("click", () => closeWorkspace(ws));
    } else {
      li.querySelector<HTMLElement>(".open-btn")!.addEventListener("click", () => openWorkspace(ws));
    }
    li.querySelector<HTMLElement>(".delete-btn")!.addEventListener("click", () => deleteWorkspace(ws));
    sitesListEl.appendChild(li);
  }
}

function kindLabel(kind: string): string {
  switch (kind) {
    case "local-folder": return "Managed folder";
    case "local-import": return "Existing folder";
    case "remote": return "Remote server";
    default: return kind;
  }
}

/// Convert a DB row (snake_case) to the camelCase shape expected by the Rust Workspace struct.
function toCamelWorkspace(ws: Workspace) {
  return {
    id: ws.id,
    name: ws.name,
    slug: ws.slug,
    kind: ws.kind,
    folderPath: ws.folder_path,
    startPage: ws.start_page,
    serverUrl: ws.server_url,
  };
}

async function openWorkspace(ws: Workspace): Promise<void> {
  try {
    await invoke("open_workspace", { workspace: toCamelWorkspace(ws) });
    const database = await loadDb();
    await database.execute("UPDATE workspaces SET last_opened_at = $1 WHERE id = $2;", [
      Math.floor(Date.now() / 1000),
      ws.id,
    ]);
    await loadWorkspaces();
  } catch (err) {
    alert(`Failed to open workspace: ${err}`);
  }
}

async function closeWorkspace(ws: Workspace): Promise<void> {
  try {
    await invoke("close_workspace", { workspaceId: ws.id });
    // give the window a moment to close before re-rendering
    setTimeout(() => loadWorkspaces(), 300);
  } catch (err) {
    alert(`Failed to close workspace: ${err}`);
  }
}

async function deleteWorkspace(ws: Workspace): Promise<void> {
  const folderNote = ws.kind === "local-folder"
    ? "\n\nThe managed folder at ~/Webforge Studio Workspaces/" + ws.slug + " will be deleted from disk."
    : "\n\nOnly the workspace record is removed (the folder on disk is left untouched).";
  if (!confirm(`Delete "${ws.name}"?${folderNote}`)) return;
  try {
    await invoke("close_workspace", { workspaceId: ws.id });
  } catch {
    // window may not be open — ignore
  }
  try {
    await invoke("delete_workspace_folder", { slug: ws.slug, folderPath: ws.folder_path, kind: ws.kind });
  } catch (err) {
    console.warn("delete_workspace_folder failed:", err);
  }
  const database = await loadDb();
  await database.execute("DELETE FROM workspace_prefs WHERE workspace_id = $1;", [ws.id]);
  await database.execute("DELETE FROM workspace_runtime WHERE workspace_id = $1;", [ws.id]);
  await database.execute("DELETE FROM workspaces WHERE id = $1;", [ws.id]);
  await loadWorkspaces();
}

async function addWorkspace(): Promise<void> {
  const name = nameInputEl.value.trim();
  const kind = kindSelectEl.value;
  statusEl.textContent = "";
  statusEl.className = "status";

  if (!name) {
    statusEl.textContent = "Name is required.";
    statusEl.className = "status error";
    return;
  }

  let folderPath: string | null = null;
  let serverUrl: string | null = null;
  let startPage = "index.html";

  if (kind === "local-import") {
    folderPath = folderPathEl.value.trim();
    if (!folderPath) {
      statusEl.textContent = "Pick an existing folder.";
      statusEl.className = "status error";
      return;
    }
    startPage = startPageEl.value.trim() || "index.html";
  } else if (kind === "local-folder") {
    // managed folder — path will be created under ~/Webforge Studio Workspaces/<slug>/
    startPage = startPageEl.value.trim() || "index.html";
  } else if (kind === "remote") {
    const url = urlInputEl.value.trim();
    if (!url) {
      statusEl.textContent = "Server URL is required.";
      statusEl.className = "status error";
      return;
    }
    if (!/^https?:\/\//i.test(url)) {
      statusEl.textContent = "URL must start with http:// or https://";
      statusEl.className = "status error";
      return;
    }
    serverUrl = url;
  }

  try {
    const database = await loadDb();
    // de-dupe slug against existing rows
    const existingRows = await database.select<{ slug: string }[]>("SELECT slug FROM workspaces;");
    const existingSlugs = existingRows.map((r) => r.slug);
    const slug = await invoke<string>("slugify", { name, existing: existingSlugs });

    let resolvedFolder: string;
    if (kind === "local-folder") {
      resolvedFolder = await invoke<string>("create_workspace_folder", { slug });
    } else if (kind === "local-import") {
      resolvedFolder = folderPath!;
    } else {
      // remote — no real folder; use a placeholder so the NOT NULL column is satisfied
      resolvedFolder = `remote-${slug}`;
    }

    await database.execute(
      "INSERT INTO workspaces (name, slug, kind, folder_path, start_page, server_url) VALUES ($1, $2, $3, $4, $5, $6);",
      [name, slug, kind, resolvedFolder, startPage, serverUrl]
    );
    statusEl.textContent = "";
    nameInputEl.value = "";
    urlInputEl.value = "";
    folderPathEl.value = "";
    startPageEl.value = "index.html";
    addFormEl.classList.add("hidden");
    await loadWorkspaces();
  } catch (err) {
    statusEl.textContent = `Failed to create workspace: ${err}`;
    statusEl.className = "status error";
  }
}

async function showServerStatus(): Promise<void> {
  try {
    const port = await invoke<number>("get_server_port");
    serverStatusEl.textContent = `Server: 127.0.0.1:${port}`;
    serverStatusEl.className = "server-status ready";
  } catch {
    serverStatusEl.textContent = "Server: not started";
    serverStatusEl.className = "server-status error";
  }
}

async function showWorkspacesRoot(): Promise<void> {
  try {
    const root = await invoke<string>("get_workspaces_root");
    workspacesRootLabelEl.textContent = `Folders: ${root}`;
  } catch {
    workspacesRootLabelEl.textContent = "";
  }
}

async function autoResumeWorkspaces(): Promise<void> {
  // On launch, re-open any workspace that was running when the app last quit.
  // (The window_label persisted in workspace_runtime refers to a window that no longer
  // exists, so we treat "running" rows as "should reopen".)
  try {
    const database = await loadDb();
    const rows = await database.select<{ workspace_id: number }[]>(
      "SELECT workspace_id FROM workspace_runtime WHERE status = 'running';"
    );
    if (rows.length === 0) return;
    // mark all as stopped first, since the actual windows are gone after a restart
    for (const r of rows) {
      await database.execute(
        "UPDATE workspace_runtime SET status = 'stopped', window_label = NULL, port = NULL, updated_at = $1 WHERE workspace_id = $2;",
        [Math.floor(Date.now() / 1000), r.workspace_id]
      );
    }
    // fetch the workspace rows and re-open them
    const ids = rows.map((r) => r.workspace_id);
    const placeholders = ids.map((_, i) => `$${i + 1}`).join(",");
    const wsRows = await database.select<Workspace[]>(
      `SELECT id, name, slug, kind, folder_path, start_page, server_url, created_at, last_opened_at FROM workspaces WHERE id IN (${placeholders});`,
      ids
    );
    for (const ws of wsRows) {
      try {
        await invoke("open_workspace", { workspace: toCamelWorkspace(ws) });
        await database.execute("UPDATE workspaces SET last_opened_at = $1 WHERE id = $2;", [
          Math.floor(Date.now() / 1000),
          ws.id,
        ]);
      } catch (err) {
        console.warn(`auto-resume failed for workspace ${ws.id}:`, err);
      }
    }
  } catch (err) {
    console.warn("auto-resume check failed:", err);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  document.querySelector("#toggle-add")!.addEventListener("click", () => {
    addFormEl.classList.toggle("hidden");
    if (!addFormEl.classList.contains("hidden")) {
      nameInputEl.focus();
      updateKindRows();
    }
  });
  document.querySelector("#cancel-add")!.addEventListener("click", () => {
    addFormEl.classList.add("hidden");
    statusEl.textContent = "";
  });
  document.querySelector("#add-site")!.addEventListener("click", addWorkspace);

  function updateKindRows() {
    const kind = kindSelectEl.value;
    if (kind === "remote") {
      localRowEl.classList.add("hidden");
      remoteRowEl.classList.remove("hidden");
    } else {
      localRowEl.classList.remove("hidden");
      remoteRowEl.classList.add("hidden");
      // for managed folders, hide the folder picker (we create it automatically)
      if (kind === "local-folder") {
        folderLabelEl.classList.add("hidden");
        startPageEl.value = startPageEl.value || "index.html";
      } else {
        folderLabelEl.classList.remove("hidden");
      }
    }
  }

  kindSelectEl.addEventListener("change", updateKindRows);

  pickFolderBtn.addEventListener("click", async () => {
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked === "string") folderPathEl.value = picked;
  });

  showServerStatus();
  showWorkspacesRoot();
  loadWorkspaces().catch((e) => {
    emptyEl.textContent = `Failed to load workspaces: ${e}`;
    emptyEl.classList.remove("hidden");
  });

  // auto-resume after the initial render so the user sees their cards first
  setTimeout(() => {
    autoResumeWorkspaces().then(() => loadWorkspaces()).catch(() => {});
  }, 600);

  document.querySelectorAll<HTMLButtonElement>(".tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      const view = tab.dataset.view;
      document.querySelectorAll(".tab").forEach((t) => t.classList.remove("active"));
      tab.classList.add("active");
      document.querySelectorAll(".view").forEach((v) => v.classList.add("hidden"));
      document.querySelector(`#view-${view}`)!.classList.remove("hidden");
      document.querySelector(`#view-${view}`)!.classList.add("active");
    });
  });

  initThemeBuilder();
  initConnectAgent();
});

// ---- Connect Agent tab (stub — Phase D fills in real MCP control) ----
function initConnectAgent(): void {
  const startBtn = document.querySelector<HTMLButtonElement>("#mcp-start")!;
  const stopBtn = document.querySelector<HTMLButtonElement>("#mcp-stop")!;
  const statusEl = document.querySelector<HTMLElement>("#mcp-status")!;
  const configEl = document.querySelector<HTMLElement>("#mcp-config")!;
  const snippetEl = configEl.querySelector<HTMLElement>(".config-snippet")!;
  const copyBtn = document.querySelector<HTMLButtonElement>("#mcp-copy")!;
  const activeEditorEl = document.querySelector<HTMLElement>("#mcp-active-editor")!;

  startBtn.addEventListener("click", () => {
    statusEl.textContent = "MCP server start: coming in Phase C/D (Rust-native MCP)";
    statusEl.className = "status warning";
  });
  stopBtn.addEventListener("click", () => {
    statusEl.textContent = "Not running";
    statusEl.className = "status";
  });
  copyBtn.addEventListener("click", () => {
    navigator.clipboard.writeText(snippetEl.textContent || "").then(() => {
      copyBtn.textContent = "Copied!";
      setTimeout(() => (copyBtn.textContent = "Copy"), 1200);
    });
  });
  activeEditorEl.textContent = "No editor window open.";
}