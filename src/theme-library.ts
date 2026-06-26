import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

interface ThemeCard {
  slug: string;
  name: string;
  groupName: string;
  sectionCount: number;
  createdAt: number;
  sourceFolder: string | null;
  kind: string | null;
  thumbnail: string | null;
}

const gridEl = document.querySelector<HTMLElement>("#lib-grid")!;
const emptyEl = document.querySelector<HTMLElement>("#lib-empty")!;
const refreshBtn = document.querySelector<HTMLElement>("#lib-refresh")!;
const importZipBtn = document.querySelector<HTMLElement>("#lib-import-zip")!;

export async function loadThemes(): Promise<void> {
  try {
    const themes = await invoke<ThemeCard[]>("list_themes");
    renderThemes(themes);
  } catch (e) {
    emptyEl.textContent = `Failed to load themes: ${e}`;
    emptyEl.classList.remove("hidden");
  }
}

function renderThemes(themes: ThemeCard[]): void {
  gridEl.innerHTML = "";
  if (themes.length === 0) {
    emptyEl.classList.remove("hidden");
    gridEl.classList.add("hidden");
    return;
  }
  emptyEl.classList.add("hidden");
  gridEl.classList.remove("hidden");

  for (const theme of themes) {
    const card = document.createElement("div");
    card.className = "theme-card";

    const thumbEl = theme.thumbnail
      ? `<img class="theme-thumb" src="${theme.thumbnail}" alt="${theme.name}" />`
      : `<div class="theme-thumb-placeholder">No preview</div>`;

    const dateStr = new Date(theme.createdAt * 1000).toLocaleDateString();
    const kindBadge = theme.kind ? `<span class="badge-mini kind-${theme.kind}">${theme.kind}</span>` : "";

    card.innerHTML = `
      <div class="theme-thumb-container">${thumbEl}</div>
      <div class="theme-card-body">
        <div class="theme-card-name"></div>
        <div class="theme-card-meta">
          <span class="badge-mini">${theme.sectionCount} sections</span>
          ${kindBadge}
          <span class="muted">${dateStr}</span>
        </div>
      </div>
      <div class="theme-card-actions">
        <button class="btn primary preview-btn">Preview</button>
        <button class="btn danger delete-btn">Delete</button>
      </div>
    `;
    card.querySelector<HTMLElement>(".theme-card-name")!.textContent = theme.name;

    card.querySelector<HTMLElement>(".preview-btn")!.addEventListener("click", () => previewTheme(theme));
    card.querySelector<HTMLElement>(".delete-btn")!.addEventListener("click", () => deleteTheme(theme));
    gridEl.appendChild(card);
  }
}

async function previewTheme(theme: ThemeCard): Promise<void> {
  try {
    await invoke("preview_theme", { slug: theme.slug });
  } catch (e) {
    alert(`Failed to preview: ${e}`);
  }
}

async function deleteTheme(theme: ThemeCard): Promise<void> {
  if (!confirm(`Delete theme "${theme.name}"? This removes it from the library.`)) return;
  try {
    await invoke("delete_theme", { slug: theme.slug });
    await loadThemes();
  } catch (e) {
    alert(`Failed to delete: ${e}`);
  }
}

async function importZip(): Promise<void> {
  const picked = await openDialog({
    multiple: false,
    filters: [{ name: "Zip", extensions: ["zip"] }],
  });
  if (typeof picked !== "string") return;
  try {
    const path = await invoke<string>("import_theme_zip", { zipPath: picked });
    console.log("Imported theme to:", path);
    await loadThemes();
  } catch (e) {
    alert(`Import failed: ${e}`);
  }
}

export function initThemeLibrary(): void {
  refreshBtn.addEventListener("click", loadThemes);
  importZipBtn.addEventListener("click", importZip);
  loadThemes();
}