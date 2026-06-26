import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import html2canvas from "html2canvas";

interface FileEntry {
  path: string;
  relative_path: string;
  is_dir: boolean;
  size: number;
  ext: string;
}

interface FolderScan {
  root: string;
  entries: FileEntry[];
  html_files: FileEntry[];
  php_files: FileEntry[];
  css_files: FileEntry[];
  js_files: FileEntry[];
  asset_files: FileEntry[];
}

interface Classification {
  kind: string;
  theme_name: string | null;
  theme_version: string | null;
  template_count: number;
  notes: string[];
}

interface GeneratedSection {
  key: string;
  name: string;
  source_file: string;
  html: string;
  tag: string;
  class: string;
  has_assets: boolean;
  screenshot: number[] | null;
}

interface ConversionResult {
  sections: GeneratedSection[];
  sections_js: string;
  group_name: string;
  asset_paths: string[];
  asset_rewrites: [string, string][];
  warnings: string[];
}

interface AiSection {
  key: string;
  name: string;
  tag: string;
  html: string;
  source_file: string;
}

interface AiConversionResult {
  sections: AiSection[];
  sections_js: string;
  group_name: string;
  asset_paths: string[];
  warnings: string[];
  reasoning: string;
}

interface AiSettings {
  provider: string;
  baseUrl: string;
  apiKey: string;
  model: string;
}

interface AiProgress {
  step: string;
  message: string;
  percent: number;
}

interface ScreenshotEntry {
  key: string;
  filename: string;
  bytes: number[];
}

interface ExportInput {
  scan: FolderScan;
  sections_js: string;
  group_name: string;
  asset_paths: string[];
  output_path: string;
  screenshots: ScreenshotEntry[];
}

interface ExportResult {
  path: string;
  bytes_written: number;
  file_count: number;
}

const state: {
  scan: FolderScan | null;
  classification: Classification | null;
  conversion: ConversionResult | null;
  screenshots: ScreenshotEntry[];
  aiSettings: AiSettings | null;
} = { scan: null, classification: null, conversion: null, screenshots: [], aiSettings: null };

export function initThemeBuilder(): void {
  const dropEl = document.querySelector<HTMLElement>("#tb-drop")!;
  const dropInner = document.querySelector<HTMLElement>("#tb-drop-inner")!;
  const classifyEl = document.querySelector<HTMLElement>("#tb-classify")!;
  const convertEl = document.querySelector<HTMLElement>("#tb-convert")!;
  const resultsEl = document.querySelector<HTMLElement>("#tb-results")!;
  const groupNameEl = document.querySelector<HTMLInputElement>("#tb-group-name")!;
  const wpUrlEl = document.querySelector<HTMLInputElement>("#tb-wp-url")!;
  const wpRowEl = document.querySelector<HTMLElement>("#tb-wp-row")!;
  const exportBtn = document.querySelector<HTMLElement>("#tb-export")!;
  const saveLibraryBtn = document.querySelector<HTMLElement>("#tb-save-library")!;
  const screenshotBtn = document.querySelector<HTMLElement>("#tb-screenshots")!;
  const logEl = document.querySelector<HTMLElement>("#tb-log")!;

  // AI settings elements
  const aiSettingsBtn = document.querySelector<HTMLElement>("#tb-ai-settings-btn")!;
  const aiSettingsPanel = document.querySelector<HTMLElement>("#tb-ai-settings")!;
  const aiProviderSelect = document.querySelector<HTMLSelectElement>("#ai-provider")!;
  const aiModelEl = document.querySelector<HTMLInputElement>("#ai-model")!;
  const aiBaseUrlEl = document.querySelector<HTMLInputElement>("#ai-base-url")!;
  const aiApiKeyEl = document.querySelector<HTMLInputElement>("#ai-api-key")!;
  const aiSaveBtn = document.querySelector<HTMLElement>("#ai-save")!;
  const aiTestBtn = document.querySelector<HTMLElement>("#ai-test")!;
  const aiSettingsStatus = document.querySelector<HTMLElement>("#ai-settings-status")!;

  // AI convert elements
  const convertAiBtn = document.querySelector<HTMLElement>("#tb-convert-ai-btn")!;
  const aiProgressPanel = document.querySelector<HTMLElement>("#tb-ai-progress")!;
  const aiProgressFill = document.querySelector<HTMLElement>("#ai-progress-fill")!;
  const aiProgressText = document.querySelector<HTMLElement>("#ai-progress-text")!;
  const aiReasoningEl = document.querySelector<HTMLElement>("#ai-reasoning")!;

  function log(msg: string, level: "info" | "warn" | "error" = "info"): void {
    const line = document.createElement("div");
    line.className = `log-${level}`;
    line.textContent = msg;
    logEl.appendChild(line);
    logEl.scrollTop = logEl.scrollHeight;
  }

  function reset(): void {
    state.scan = null;
    state.classification = null;
    state.conversion = null;
    state.screenshots = [];
    classifyEl.classList.add("hidden");
    convertEl.classList.add("hidden");
    resultsEl.classList.add("hidden");
    exportBtn.classList.add("hidden");
    saveLibraryBtn.classList.add("hidden");
    screenshotBtn.classList.add("hidden");
    logEl.innerHTML = "";
  }

  async function handleFolder(path: string): Promise<void> {
    reset();
    log(`Scanning ${path}...`);
    try {
      const scan = await invoke<FolderScan>("scan_folder", { folderPath: path });
      state.scan = scan;
      log(`Found ${scan.html_files.length} HTML, ${scan.php_files.length} PHP, ${scan.css_files.length} CSS, ${scan.asset_files.length} asset files`);
      const classification = await invoke<Classification>("classify_folder", { scan });
      state.classification = classification;
      renderClassification(classification);
      classifyEl.classList.remove("hidden");
      convertEl.classList.remove("hidden");
      if (classification.theme_name) groupNameEl.value = classification.theme_name;
      // show WP URL field only for WP themes
      if (classification.kind === "wordpress") {
        wpRowEl.classList.remove("hidden");
      } else {
        wpRowEl.classList.add("hidden");
      }
    } catch (e) {
      log(`Error: ${e}`, "error");
    }
  }

  function renderClassification(c: Classification): void {
    const kindBadge = c.kind === "wordpress" ? "WordPress theme" : c.kind === "html" ? "Static HTML" : "Unknown";
    const themeInfo = c.theme_name ? `: ${c.theme_name}${c.theme_version ? ` v${c.theme_version}` : ""}` : "";
    classifyEl.innerHTML = `
      <div class="classify-row">
        <span class="badge badge-${c.kind}">${kindBadge}${themeInfo}</span>
        <span class="muted">${c.template_count} templates detected</span>
      </div>
      <ul class="notes">${c.notes.map((n) => `<li>${n}</li>`).join("")}</ul>
    `;
  }

  async function convert(): Promise<void> {
    if (!state.scan || !state.classification) return;
    const groupName = groupNameEl.value.trim() || "imported";
    const wpUrl = wpUrlEl.value.trim() || null;
    const kind = state.classification.kind;
    if (kind === "wordpress" && wpUrl) {
      log(`Converting WordPress theme with live render from ${wpUrl} (group: ${groupName})...`);
    } else if (kind === "wordpress") {
      log(`Converting WordPress theme via PHP parsing (no WP URL provided, group: ${groupName})...`);
    } else {
      log(`Converting as ${kind} (group: ${groupName})...`);
    }
    try {
      const result = await invoke<ConversionResult>("convert_template", {
        scan: state.scan,
        groupName,
        kind,
        wpUrl,
      });
      state.conversion = result;
      state.screenshots = [];
      for (const w of result.warnings) log(w, "warn");
      log(`Generated ${result.sections.length} sections, ${result.asset_paths.length} asset references`);
      if (result.asset_rewrites.length > 0) {
        log(`Rewrote ${result.asset_rewrites.length} asset paths to assets/...`);
      }
      renderResults(result);
      resultsEl.classList.remove("hidden");
      exportBtn.classList.remove("hidden");
      saveLibraryBtn.classList.remove("hidden");
      screenshotBtn.classList.remove("hidden");
    } catch (e) {
      log(`Conversion failed: ${e}`, "error");
    }
  }

  // ---- AI settings management ----
  async function loadAiSettings(): Promise<void> {
    try {
      const presets = await invoke<[string, string, string][]>("get_provider_presets");
      aiProviderSelect.innerHTML = '<option value="">Select a provider...</option>' +
        presets.map(([name, url, model]) =>
          `<option value="${name}" data-url="${url}" data-model="${model}">${name}</option>`
        ).join("");
      const saved = await invoke<AiSettings | null>("load_ai_settings");
      if (saved) {
        state.aiSettings = saved;
        aiProviderSelect.value = saved.provider;
        aiBaseUrlEl.value = saved.baseUrl;
        aiApiKeyEl.value = saved.apiKey;
        aiModelEl.value = saved.model;
      }
    } catch (e) {
      aiSettingsStatus.textContent = `Failed to load AI settings: ${e}`;
      aiSettingsStatus.className = "status error";
    }
  }

  function setupAiSettingsListeners(): void {
    aiSettingsBtn.addEventListener("click", () => {
      aiSettingsPanel.classList.toggle("hidden");
    });

    aiProviderSelect.addEventListener("change", () => {
      const opt = aiProviderSelect.selectedOptions[0];
      if (opt) {
        const url = opt.dataset.url || "";
        const model = opt.dataset.model || "";
        if (url) aiBaseUrlEl.value = url;
        if (model) aiModelEl.value = model;
      }
    });

    aiSaveBtn.addEventListener("click", async () => {
      const settings: AiSettings = {
        provider: aiProviderSelect.value,
        baseUrl: aiBaseUrlEl.value.trim(),
        apiKey: aiApiKeyEl.value.trim(),
        model: aiModelEl.value.trim(),
      };
      if (!settings.baseUrl || !settings.apiKey || !settings.model) {
        aiSettingsStatus.textContent = "Base URL, API key, and model are required.";
        aiSettingsStatus.className = "status error";
        return;
      }
      try {
        await invoke("save_ai_settings", { settings });
        state.aiSettings = settings;
        aiSettingsStatus.textContent = "Saved.";
        aiSettingsStatus.className = "status";
      } catch (e) {
        aiSettingsStatus.textContent = `Save failed: ${e}`;
        aiSettingsStatus.className = "status error";
      }
    });

    aiTestBtn.addEventListener("click", async () => {
      if (!aiBaseUrlEl.value.trim() || !aiApiKeyEl.value.trim() || !aiModelEl.value.trim()) {
        aiSettingsStatus.textContent = "Enter Base URL, API key, and model first.";
        aiSettingsStatus.className = "status error";
        return;
      }
      aiSettingsStatus.textContent = "Settings look valid. Try 'Convert with AI' to test the full connection.";
      aiSettingsStatus.className = "status";
    });
  }

  // ---- AI-powered conversion ----
  async function convertWithAi(): Promise<void> {
    if (!state.scan || !state.classification) return;

    // load settings from the form (in case user didn't save)
    let settings = state.aiSettings;
    if (!settings) {
      settings = {
        provider: aiProviderSelect.value,
        baseUrl: aiBaseUrlEl.value.trim(),
        apiKey: aiApiKeyEl.value.trim(),
        model: aiModelEl.value.trim(),
      };
    }
    if (!settings.baseUrl || !settings.apiKey || !settings.model) {
      log("AI settings required. Click 'AI Settings' to configure your provider.", "error");
      aiSettingsPanel.classList.remove("hidden");
      return;
    }

    const groupName = groupNameEl.value.trim() || "imported";
    const kind = state.classification.kind;

    log(`Starting AI conversion (${settings.provider}/${settings.model})...`);
    aiProgressPanel.classList.remove("hidden");
    aiProgressFill.style.width = "0%";
    aiProgressText.textContent = "Initializing...";
    aiReasoningEl.classList.add("hidden");

    try {
      const result = await invoke<AiConversionResult>("convert_with_ai", {
        scan: state.scan,
        groupName,
        kind,
        settings,
      });

      // map AI sections to the same shape as mechanical conversion for the UI
      const mapped: ConversionResult = {
        sections: result.sections.map((s) => ({
          key: s.key,
          name: s.name,
          source_file: s.source_file,
          html: s.html,
          tag: s.tag,
          class: "",
          has_assets: /<(img|video)|background-image/i.test(s.html),
          screenshot: null,
        })),
        sections_js: result.sections_js,
        group_name: result.group_name,
        asset_paths: result.asset_paths,
        asset_rewrites: [],
        warnings: result.warnings,
      };
      state.conversion = mapped;
      state.screenshots = [];

      for (const w of result.warnings) log(w, "warn");
      log(`AI generated ${result.sections.length} sections`);
      if (result.reasoning) {
        aiReasoningEl.innerHTML = `<strong>AI reasoning:</strong> ${result.reasoning}`;
        aiReasoningEl.classList.remove("hidden");
        log(`AI reasoning: ${result.reasoning}`);
      }
      renderResults(mapped);
      resultsEl.classList.remove("hidden");
      exportBtn.classList.remove("hidden");
      saveLibraryBtn.classList.remove("hidden");
      screenshotBtn.classList.remove("hidden");
    } catch (e) {
      log(`AI conversion failed: ${e}`, "error");
      aiProgressText.textContent = `Failed: ${e}`;
    }
  }

  // listen for AI progress events
  function setupAiProgressListener(): void {
    listen<AiProgress>("ai-progress", (event) => {
      const p = event.payload;
      if (p) {
        aiProgressFill.style.width = p.percent + "%";
        aiProgressText.textContent = p.message;
        if (p.percent >= 100) {
          setTimeout(() => {
            aiProgressFill.style.width = "0%";
          }, 1000);
        }
      }
    }).catch(() => {});
  }

  function renderResults(result: ConversionResult): void {
    const list = result.sections
      .map(
        (s) => `
        <li class="section-result" data-key="${s.key}">
          <div class="section-meta">
            <span class="section-key"></span>
            <span class="muted">from <code></code></span>
            <span class="badge-mini tag-${s.tag}">${s.tag}</span>
            ${s.class ? `<span class="badge-mini">.${s.class}</span>` : ""}
            ${s.has_assets ? '<span class="badge-mini assets">assets</span>' : ""}
          </div>
          <div class="section-name"></div>
          <div class="section-thumb-container"></div>
          <pre class="section-preview"></pre>
        </li>`
      )
      .join("");
    resultsEl.innerHTML = `<h3>Generated sections (${result.sections.length})</h3><ul class="section-list">${list}</ul>`;
    const items = resultsEl.querySelectorAll<HTMLLIElement>(".section-result");
    result.sections.forEach((s, i) => {
      const el = items[i];
      el.querySelector<HTMLElement>(".section-key")!.textContent = s.key;
      el.querySelector<HTMLElement>("code")!.textContent = s.source_file;
      el.querySelector<HTMLElement>(".section-name")!.textContent = s.name;
      const pre = el.querySelector<HTMLPreElement>(".section-preview")!;
      pre.textContent = s.html.length > 600 ? s.html.slice(0, 600) + "\n... (truncated)" : s.html;
    });
  }

  // ---- screenshot generation via html2canvas ----
  async function generateScreenshots(): Promise<void> {
    if (!state.conversion) return;
    const sections = state.conversion.sections;
    if (sections.length === 0) return;
    log(`Generating ${sections.length} screenshots...`);
    const screenshots: ScreenshotEntry[] = [];
    const screenshotFilenames: [string, string][] = [];

    for (let i = 0; i < sections.length; i++) {
      const s = sections[i];
      const filename = `${s.key.replace(/\//g, "-")}.png`;
      try {
        // render the section HTML in an offscreen container, then capture with html2canvas
        const container = document.createElement("div");
        container.style.cssText = "position:fixed;left:-9999px;top:0;width:1200px;min-height:200px;background:#ffffff;padding:20px;pointer-events:none;";
        container.innerHTML = s.html;
        document.body.appendChild(container);
        // wait a tick for layout
        await new Promise((r) => setTimeout(r, 50));
        const canvas = await html2canvas(container, {
          width: 1200,
          height: Math.min(container.scrollHeight + 40, 800),
          backgroundColor: "#ffffff",
          logging: false,
          useCORS: true,
          scale: 1,
        });
        document.body.removeChild(container);
        const blob = await new Promise<Blob | null>((resolve) =>
          canvas.toBlob(resolve, "image/png")
        );
        if (!blob) {
          log(`  ${i + 1}/${sections.length} ${s.key}: screenshot failed (no blob)`, "warn");
          continue;
        }
        const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
        screenshots.push({ key: s.key, filename, bytes });
        screenshotFilenames.push([s.key, `screenshots/${filename}`]);
        // show thumbnail in the UI
        const item = resultsEl.querySelector<HTMLElement>(`[data-key="${s.key}"]`);
        if (item) {
          const thumbContainer = item.querySelector<HTMLElement>(".section-thumb-container")!;
          thumbContainer.innerHTML = "";
          const img = document.createElement("img");
          img.src = canvas.toDataURL("image/png");
          img.className = "section-thumb";
          thumbContainer.appendChild(img);
        }
        log(`  ${i + 1}/${sections.length} ${s.key}: ${bytes.length} bytes`);
      } catch (e) {
        log(`  ${i + 1}/${sections.length} ${s.key}: screenshot failed - ${e}`, "warn");
      }
    }

    state.screenshots = screenshots;
    log(`Generated ${screenshots.length}/${sections.length} screenshots`);

    // re-render sections.js with screenshot paths
    if (screenshots.length > 0) {
      try {
        const updatedJs = await invoke<string>("render_with_screenshots", {
          groupName: state.conversion.group_name,
          sections: state.conversion.sections,
          screenshots: screenshotFilenames,
        });
        state.conversion.sections_js = updatedJs;
        log("sections.js updated with screenshot paths");
      } catch (e) {
        log(`Failed to update sections.js with screenshots: ${e}`, "warn");
      }
    }
  }

  async function doExport(): Promise<void> {
    if (!state.scan || !state.conversion) return;
    const defaultName = `webforge-${state.conversion.group_name}-${Date.now()}.zip`;
    const path = await saveDialog({
      defaultPath: defaultName,
      filters: [{ name: "Zip", extensions: ["zip"] }],
    });
    if (!path) return;
    log(`Exporting to ${path}...`);
    try {
      const res = await invoke<ExportResult>("export_zip", {
        input: {
          scan: state.scan,
          sections_js: state.conversion.sections_js,
          group_name: state.conversion.group_name,
          asset_paths: state.conversion.asset_paths,
          output_path: path,
          screenshots: state.screenshots,
        } as ExportInput,
      });
      log(`Exported ${res.file_count} files, ${Math.round(res.bytes_written / 1024)} KB`);
    } catch (e) {
      log(`Export failed: ${e}`, "error");
    }
  }

  async function saveToLibrary(): Promise<void> {
    if (!state.scan || !state.conversion) return;
    const groupName = state.conversion.group_name;
    const slug = groupName.toLowerCase().replace(/[^a-z0-9-]/g, "-").replace(/-+/g, "-").replace(/^-|-$/g, "") || "theme";
    log("Saving to Theme Library...");
    try {
      const screenshots = state.screenshots.map((s) => s.bytes);
      await invoke("save_theme_to_library", {
        slug,
        name: groupName,
        groupName,
        sectionCount: state.conversion.sections.length,
        sourceFolder: state.scan.root,
        kind: state.classification?.kind || null,
        sectionsJs: state.conversion.sections_js,
        blocksJs: null,
        screenshots,
        assetFiles: state.conversion.asset_paths,
        scanRoot: state.scan.root,
      });
      log(`Saved "${groupName}" to library (${state.conversion.sections.length} sections)`);
    } catch (e) {
      log(`Save to library failed: ${e}`, "error");
    }
  }

  // ---- event wiring ----
  dropEl.addEventListener("click", async () => {
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked === "string") await handleFolder(picked);
  });

  ["dragenter", "dragover"].forEach((ev) =>
    dropEl.addEventListener(ev, (e) => {
      e.preventDefault();
      dropEl.classList.add("drag-active");
    })
  );
  ["dragleave", "drop"].forEach((ev) =>
    dropEl.addEventListener(ev, (e) => {
      e.preventDefault();
      dropEl.classList.remove("drag-active");
    })
  );
  dropEl.addEventListener("drop", async () => {
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked === "string") await handleFolder(picked);
  });

  document.querySelector<HTMLElement>("#tb-convert-btn")!.addEventListener("click", convert);
  convertAiBtn.addEventListener("click", convertWithAi);
  screenshotBtn.addEventListener("click", generateScreenshots);
  exportBtn.addEventListener("click", doExport);
  saveLibraryBtn.addEventListener("click", saveToLibrary);

  // AI settings
  setupAiSettingsListeners();
  loadAiSettings();
  setupAiProgressListener();

  void dropInner;
}