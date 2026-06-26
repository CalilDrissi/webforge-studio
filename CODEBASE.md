# Webforge Studio — Codebase Reference Document

**Version:** 0.1.0
**Date:** June 2026
**Repository:** https://github.com/CalilDrissi/webforge-studio
**License:** Apache-2.0 (editor); see `src-tauri/resources/webforge/LICENSE`

---

## 1. What is Webforge Studio?

Webforge Studio is a **desktop application** (macOS + Windows) that wraps a whitelabelled fork of VVVEB.js — a visual drag-and-drop HTML page builder — and adds a multi-site workspace manager, a Theme Builder for converting HTML/WordPress templates into editable sections, an AI agent for autonomous template conversion, and an MCP (Model Context Protocol) server for connecting external AI clients to the live editor.

The app is built with **Tauri v2** (Rust backend + TypeScript/HTML frontend) and ships as a single binary that embeds:
- A **Vite + vanilla TypeScript** dashboard (the launcher UI)
- A **Hyper HTTP server** (Rust) that serves the bundled Webforge editor and replaces the original PHP backend (`save.php`, `upload.php`, `scan.php`, `editor.php`) with native Rust route handlers
- A **SQLite database** for workspace metadata, per-workspace preferences, and runtime state
- The **full Webforge editor** (HTML/CSS/JS, ~74 MB of resources bundled inside the app)

---

## 2. Technology Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| Desktop shell | Tauri v2 | WebView window management, IPC commands, native dialogs, file system access |
| Backend | Rust (edition 2021) | Embedded HTTP server, workspace management, file I/O, AI conversion, export |
| Frontend | TypeScript + Vite 6 | Dashboard SPA, Theme Builder UI |
| Embedded editor | Vanilla JS (VVVEB.js fork) | Drag-and-drop HTML page builder |
| Database | SQLite (via tauri-plugin-sql) | Workspace CRUD, preferences, runtime state |
| HTTP server | Hyper 1.x + Tokio | Serves editor assets, `/api/*` routes, remote proxy |
| AI integration | OpenAI-compatible `/v1/chat/completions` | Template conversion + in-editor sidebar assistant |
| Build/CI | GitHub Actions | CI tests, release builds (DMG + EXE) |

### Key Rust dependencies

| Crate | Version | Role |
|-------|---------|------|
| `tauri` | 2 | App framework, IPC, window management |
| `hyper` | 1.10 | Embedded HTTP server |
| `hyper-util` | 0.1 | Server helpers (auto, graceful, tokio) |
| `http-body-util` | 0.1 | `Full<Bytes>` body type |
| `tokio` | 1.52 (full) | Async runtime for the HTTP server |
| `reqwest` | 0.13 | Remote site proxying, AI API calls, WordPress live-render |
| `serde` / `serde_json` | 1 | Serialization for IPC and JSON responses |
| `tauri-plugin-sql` | 2.4 (sqlite) | SQLite migrations + JS-side DB access |
| `tauri-plugin-fs` | 2.5 | Native file system dialogs |
| `tauri-plugin-dialog` | 2.7 | Open/save dialogs |
| `zip` | 8.6 | Theme Builder zip export |
| `regex` | 1 | PHP tag detection in saved HTML |
| `urlencoding` | 2.1 | URL encoding for editor query params |
| `dirs-next` | 2.0 | Home directory resolution for workspace paths |

### Key npm dependencies

| Package | Role |
|---------|------|
| `@tauri-apps/api` | Core IPC (`invoke`), events (`listen`) |
| `@tauri-apps/plugin-sql` | SQLite access from TypeScript |
| `@tauri-apps/plugin-dialog` | Native open/save dialogs |
| `@tauri-apps/plugin-fs` | File system access |
| `html2canvas` | Screenshot generation in Theme Builder |
| `vite` | Dev server + production build |
| `typescript` | Type checking |

---

## 3. Project Structure

```
webforge-studio/
├── index.html                     # Dashboard SPA shell (3 tabs)
├── package.json                   # Frontend deps + scripts
├── vite.config.ts                 # Vite config (port 1420, Tauri dev host)
├── tsconfig.json                  # TypeScript config
│
├── src/                           # Frontend TypeScript
│   ├── main.ts                    # Dashboard logic (workspace CRUD, tabs, auto-resume)
│   ├── theme-builder.ts           # Theme Builder tab (scan → classify → convert → export)
│   ├── styles.css                 # Dark theme CSS (465 lines)
│   └── assets/                    # Static assets
│
├── dist/                          # Built frontend (output of `npm run build`)
│   ├── index.html
│   └── assets/
│       ├── index-*.js             # Bundled dashboard JS
│       └── index-*.css            # Bundled dashboard CSS
│
├── src-tauri/                     # Rust backend
│   ├── Cargo.toml                 # Rust dependencies
│   ├── Cargo.lock                 # Locked dependency versions
│   ├── tauri.conf.json            # Tauri config (window size, plugins, bundle settings)
│   ├── build.rs                   # Tauri build script
│   ├── .cargo/config.toml         # Per-target link flags (macOS deployment target 12.0)
│   ├── capabilities/
│   │   └── default.json           # Tauri permissions (SQL, FS, dialog, opener)
│   ├── icons/                     # App icons (icns, ico, png)
│   ├── resources/
│   │   └── webforge/              # Bundled Webforge editor (~74 MB)
│   │       ├── editor.html        # Editor entry point
│   │       ├── libs/              # Editor JS libraries (builder, media, inputs, etc.)
│   │       ├── demo/              # Demo site pages (compiled + src templates)
│   │       ├── css/, js/, scss/   # Editor styles and scripts
│   │       ├── fonts/, img/, media/  # Static assets
│   │       ├── webforge-bridge.js     # Injected shim: reroutes PHP calls to /api/*
│   │       ├── webforge-ai-sidebar.js # Injected AI chat sidebar
│   │       ├── save.php / upload.php / scan.php / editor.php  # Original PHP (unused, kept for reference)
│   │       └── ...
│   └── src/
│       ├── main.rs                # Binary entry point (6 lines)
│       ├── lib.rs                 # App wiring, plugin registration, command registry, migrations
│       ├── server.rs              # Embedded HTTP server + /api/* routes (1522 lines)
│       ├── workspace.rs           # Workspace folder management + editor window lifecycle
│       ├── site_io.rs             # Save/upload Tauri commands (local + remote, conflict detection)
│       ├── ai_agent.rs            # AI-powered template conversion
│       ├── converter.rs           # Mechanical (deterministic) template conversion
│       ├── folder.rs              # Folder scanning + WordPress/HTML classification
│       └── export.rs              # Zip exporter for converted sections
│
├── .github/workflows/
│   ├── ci.yml                     # CI: typecheck + tests + release build (macOS + Windows)
│   └── release.yml                # Release: build DMG + EXE, upload to GitHub Releases
│
└── .gitignore                     # Ignores node_modules, dist, src-tauri/target
```

**Total lines of code:** ~5,500 (Rust ~3,800 + TypeScript ~1,000 + HTML/CSS ~700)

---

## 4. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                    Webforge Studio Binary                     │
│                                                               │
│  ┌──────────────┐    ┌──────────────────────────────────────┐│
│  │  Tauri Shell  │    │         Embedded HTTP Server          ││
│  │  (WebView)    │    │         (Hyper, port: random)         ││
│  │               │    │                                       ││
│  │  ┌─────────┐  │    │  /editor.html  → editor + bridge shim ││
│  │  │Dashboard│  │    │  /api/save     → handle_save()        ││
│  │  │  (Vite) │  │    │  /api/upload   → handle_upload()      ││
│  │  └─────────┘  │    │  /api/scan     → handle_scan()        ││
│  │               │    │  /api/pages    → handle_pages()       ││
│  │  ┌─────────┐  │    │  /site/<id>/   → workspace files      ││
│  │  │ Editor   │  │    │  /proxy/<url>  → remote site proxy   ││
│  │  │ Windows  │  │    │  /assets/*     → editor static files ││
│  │  │ (WebView)│  │    └──────────────────────────────────────┘│
│  │  └─────────┘  │                                           │
│  └───────┬───────┘    ┌──────────────────────────────────────┐│
│          │            │           SQLite Database              ││
│          │  IPC       │  workspaces | workspace_prefs          ││
│          │  (invoke)  │  workspace_runtime | sites (legacy)    ││
│          └───────────►│                                       │
│                       └──────────────────────────────────────┘│
│                                                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              ~/Webforge Studio Workspaces/                │ │
│  │  ├── my-site/          (managed: created by app)          │ │
│  │  │   ├── index.html     (editable in editor)              │ │
│  │  │   └── uploads/       (uploaded media)                  │ │
│  │  └── imported-site/    (existing: pointed at by user)     │ │
│  └─────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Data flow

1. **Dashboard** (Tauri WebView, loaded from Vite in dev or embedded `dist/` in production) manages workspaces via Tauri IPC commands → SQLite + filesystem
2. **Editor windows** (separate Tauri WebViews, loaded from the embedded HTTP server) open per-workspace, pointed at `http://127.0.0.1:<port>/editor.html?site=<id>&kind=<local|remote>&...`
3. The **embedded HTTP server** serves editor assets from the bundled `resources/webforge/` directory, handles `/api/*` routes for save/upload/scan (replacing PHP), and serves workspace files from `/site/<id>/`
4. The **bridge shim** (`webforge-bridge.js`) is injected into `editor.html` before `</body>` and monkey-patches `Webforge.Builder.saveAjax` and `window.fetch` to route save/upload/scan calls to `/api/*` instead of PHP endpoints
5. The **AI sidebar** (`webforge-ai-sidebar.js`) is also injected and adds a chat panel that calls an external OpenAI-compatible API directly via `fetch()`

---

## 5. Database Schema

SQLite database at `~/Library/Application Support/com.webforge.studio/webforge.db` (macOS) or equivalent on Windows. Managed via `tauri-plugin-sql` with 4 migrations.

### Migration history

| Version | Description | What it does |
|---------|-------------|-------------|
| 1 | `create_sites_table` | Creates the original `sites` table |
| 2 | `add_kind_and_paths` | Adds `kind`, `folder_path`, `server_url`, `start_page` columns to `sites` |
| 3 | `create_workspaces_tables` | Creates `workspaces`, `workspace_prefs`, `workspace_runtime` tables + indexes |
| 4 | `migrate_sites_to_workspaces` | Migrates legacy `sites` rows into `workspaces` (local → local-import, remote → remote) |

### `workspaces` table

```sql
CREATE TABLE workspaces (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,          -- URL-safe, de-duplicated
    kind TEXT NOT NULL DEFAULT 'local-folder',  -- "local-folder" | "local-import" | "remote"
    folder_path TEXT NOT NULL,          -- absolute path on disk
    start_page TEXT NOT NULL DEFAULT 'index.html',
    server_url TEXT,                    -- for remote workspaces only
    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    last_opened_at INTEGER              -- unix timestamp, NULL if never opened
);
```

### `workspace_prefs` table

```sql
CREATE TABLE workspace_prefs (
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,                -- JSON string
    PRIMARY KEY (workspace_id, key)
);
```

### `workspace_runtime` table

```sql
CREATE TABLE workspace_runtime (
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'stopped',  -- "stopped" | "running"
    window_label TEXT,                       -- Tauri window label: "workspace-<id>"
    port INTEGER,                            -- embedded HTTP server port
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    PRIMARY KEY (workspace_id)
);
```

### Workspace kinds

| Kind | Description | Folder ownership |
|------|-------------|-----------------|
| `local-folder` | Managed by Webforge Studio | Created under `~/Webforge Studio Workspaces/<slug>/`, deleted when workspace is deleted |
| `local-import` | Existing folder on disk | User-selected; not deleted on workspace deletion |
| `remote` | Remote server | No local folder; saves forwarded to remote `save.php` via Tauri command |

---

## 6. Rust Backend — Module Reference

### 6.1 `lib.rs` — App entry point and wiring

**Responsibilities:**
- Declares all submodules (`ai_agent`, `converter`, `export`, `folder`, `server`, `site_io`, `workspace`)
- Defines shared types: `SiteFolders` (Arc<RwLock<HashMap<i64, PathBuf>>>), `PortState`, `SiteFoldersState`
- Implements `run()` — the Tauri app builder:
  - Registers plugins: `opener`, `fs`, `dialog`, `sql` (with 4 migrations)
  - Resolves the bundled `resources/webforge/` directory (falls back to `CARGO_MANIFEST_DIR/resources/webforge` in dev)
  - Starts a leaked Tokio runtime + embedded HTTP server
  - Manages `PortState` and `SiteFoldersState` as Tauri state
  - Injects a JS error/unhandled-rejection logger into the main window (debug, to be removed)
  - Registers all 23 Tauri commands in one `invoke_handler!`

**Tauri commands defined here:**
- `open_site` — Legacy command for opening a site editor window (pre-workspace model)
- `get_server_port` — Returns the embedded server's port
- `register_site_folder` — Registers a folder path with the embedded server

**All registered commands:**
```
open_site, get_server_port, register_site_folder,
save_site, upload_site, get_file_mtime,
scan_folder, classify_folder,
convert_template, render_with_screenshots,
export_zip,
convert_with_ai, get_provider_presets, save_ai_settings, load_ai_settings,
slugify, create_workspace_folder, open_workspace, close_workspace,
is_workspace_open, delete_workspace_folder, get_workspaces_root, get_workspace_size
```

### 6.2 `server.rs` — Embedded HTTP server (1,522 lines)

**The largest module.** Implements a Hyper HTTP server that replaces the original PHP backend.

**Startup:**
- `start_server(root, site_folders)` — Binds to `127.0.0.1:0` (random free port), spawns a Tokio accept loop with one task per connection, returns `ServerState { root, port, site_folders }`

**Request routing (`serve_request`):**

| URL pattern | Handler | Purpose |
|-------------|---------|---------|
| `/api/*` | `handle_api_route` | PHP replacement routes (save/upload/scan/pages) |
| `/proxy/<encoded-url>` | `proxy_fetch` | Same-origin proxy for remote sites |
| `/site/<id>/<path>` | `serve_from_root` | Serve files from a registered workspace folder |
| `/editor.html` or `/editor.php` (GET) | `serve_editor_with_dynamic_pages` | Serve editor with dynamically scanned `defaultPages` |
| `/*` | `serve_from_root` | Serve static assets from bundled webforge root |

**API routes (`handle_api_route`):**

| Route | Method | Replaces | What it does |
|-------|--------|----------|-------------|
| `/api/save?site=<id>` | POST | `save.php` | Writes HTML to `<workspace>/<file>`. Supports `?action=rename`, `?action=delete`, `?action=saveReusable`, `?action=oembedProxy` |
| `/api/upload?site=<id>` | POST (multipart) | `upload.php` | Validates file extension, writes to `<workspace>/<mediaPath>/<filename>` |
| `/api/scan?site=<id>&mediaPath=<path>` | GET | `scan.php` | Returns a nested JSON tree of the media folder |
| `/api/pages?site=<id>` | GET | `editor.php` (part) | Returns a JSON array of all HTML pages in `my-pages/` and `demo/` |

**Security measures:**
- `sanitize_filename` — Strips `..`, disallows `.htaccess`/`passwd`, enforces `.html` extension, keeps only alphanumeric/`-`_`./`
- `sanitize_path_segment` — Same for upload paths, collapses `//`, strips leading slashes
- `php_tag_re` — Regex rejects `<?php`, `<? `, `<?=`, `<script language="php">` in saved HTML
- `UPLOAD_DENY_EXTENSIONS = ["php"]`, `UPLOAD_ALLOW_EXTENSIONS = ["ico","jpg","jpeg","png","gif","webp","svg"]`
- `ALLOWED_OEMBED_DOMAINS` — Whitelist for the oembed proxy (YouTube, Vimeo, X/Twitter, Reddit)
- `MAX_FILE_LIMIT = 2 MiB` — Maximum saved HTML size
- Path traversal guard in `serve_from_root` — `candidate.starts_with(root)` check

**Page scanning (`collect_pages`, `walk_html_pages`):**
- Walks `my-pages/*.html` and `demo/**/*.html`
- Excludes `editor.html`, `new-page-blank-template.html`
- Excludes any subdirectory named `src` (contains unresolved `@@include` templates)
- `index.html` in a subfolder is named after the folder (e.g., `demo/landing/index.html` → name: "landing")

**Editor serving (`serve_editor_with_dynamic_pages`):**
- Reads `editor.html` from the bundled resources
- Replaces the literal `= defaultPages;` with a dynamically scanned JS object
- Injects `webforge-bridge.js` and `webforge-ai-sidebar.js` before `</body>`

**Remote proxy (`proxy_fetch`):**
- Fetches the remote URL via reqwest (30s timeout)
- Rewrites HTML `src`/`href` attributes to route through `/proxy/`
- Rewrites CSS `url()` references
- Injects the bridge shim into proxied HTML
- Adds `X-Proxied-From` header

**Multipart parsing (`parse_multipart`):**
- Custom minimal parser (no multer dependency)
- Extracts `Content-Disposition` name + filename, value bytes
- Handles boundary delimiters correctly

**Tests:** 23 unit tests covering sanitizers, multipart, save/upload/scan/pages handlers, PHP rejection, extension validation, rename/delete actions, page exclusion.

### 6.3 `workspace.rs` — Workspace management (299 lines)

**Responsibilities:**
- Workspace folder creation under `~/Webforge Studio Workspaces/<slug>/`
- Editor window lifecycle (open, close, focus, check-open)
- Slug generation with collision de-duplication
- Workspace folder size calculation
- Safe folder deletion (only managed folders, only if path matches expected managed path)

**Tauri commands:**

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `slugify` | `name: String, existing: Vec<String>` | `String` | URL-safe slug with de-dup |
| `create_workspace_folder` | `slug: String` | `Result<String, String>` | Creates `~/Webforge Studio Workspaces/<slug>/` + starter `index.html` |
| `open_workspace` | `workspace: Workspace` | `Result<(), String>` | Registers folder with server, opens editor window (or focuses existing) |
| `close_workspace` | `workspace_id: i64` | `Result<(), String>` | Closes the editor window |
| `is_workspace_open` | `workspace_id: i64` | `bool` | Checks if editor window exists |
| `delete_workspace_folder` | `slug, folder_path, kind` | `Result<(), String>` | Deletes managed folder only (safety: path must match `workspace_folder(slug)`) |
| `get_workspaces_root` | — | `String` | Returns `~/Webforge Studio Workspaces/` |
| `get_workspace_size` | `folder_path: String` | `Result<u64, String>` | Recursive folder size in bytes |

**Editor window naming:** `workspace-<id>` (e.g., `workspace-3`)

**Editor URL format:**
```
http://127.0.0.1:<port>/editor.html?site=<id>&kind=<local|remote>&startUrl=<...>&folder=<...>&server=<...>
```

**Tests:** 6 unit tests (slugify, dedup, empty fallback, folder size, delete safety).

### 6.4 `site_io.rs` — Save/Upload commands (294 lines)

**Responsibilities:**
- Implements `save_site` and `upload_site` Tauri commands (used by the bridge shim for remote sites and as a legacy fallback)
- Local save: path traversal guard, mtime conflict detection, file write
- Local upload: write to `<folder>/uploads/<filename>`, emit progress event
- Remote save/upload: POST to remote `save.php`/`upload.php` via reqwest
- `get_file_mtime` — for conflict detection

**Conflict detection:**
- Before saving, if `expected_mtime` is provided, the current file's mtime is checked
- If mtime differs (file was modified on disk since it was loaded), the save is skipped and a `ConflictInfo` is returned
- The frontend prompts the user to overwrite or cancel

**Upload progress:**
- `upload-progress` Tauri event with `{ filename, uploaded, total, percent }`
- Local uploads emit 100% instantly (file write is synchronous)
- Remote uploads emit 0% start + 100% completion

### 6.5 `ai_agent.rs` — AI template conversion (489 lines)

**Responsibilities:**
- Converts HTML/WordPress templates into Webforge sections using an LLM
- Uses any OpenAI-compatible `/v1/chat/completions` endpoint
- Manages AI provider settings (saved to `~/.config/com.webforge.studio/ai-settings.json`)

**Provider presets:**

| Provider | Base URL | Default model |
|----------|----------|---------------|
| ollama-cloud | `https://api.ovhcloud.com/ollama/v1` | llama3.1 |
| ollama-local | `http://localhost:11434/v1` | llama3.1 |
| openai | `https://api.openai.com/v1` | gpt-4o |
| groq | `https://api.groq.com/openai/v1` | llama-3.3-70b-versatile |
| together | `https://api.together.xyz/v1` | meta-llama/Llama-3.3-70B-Instruct-Turbo |
| openrouter | `https://openrouter.ai/api/v1` | anthropic/claude-3.5-sonnet |
| custom | (user-provided) | (user-provided) |

**Conversion pipeline (`convert_with_ai`):**
1. Selects up to 10 HTML files (or PHP files for WordPress)
2. Truncates each to 8,000 characters
3. Builds a system prompt demanding `{reasoning, sections:[{name,tag,html}]}` JSON
4. Appends WordPress-specific PHP replacement rules if `kind == "wordpress"`
5. POSTs to `{baseUrl}/chat/completions` (120s timeout, temperature 0.3, Bearer auth)
6. Parses the JSON response (robust: tries direct, ```json block, ``` block, first-{-to-last-})
7. Collects asset paths from `src=`/`href=` references
8. Renders `sections.js` with `Webforge.SectionsGroup` + `Webforge.Sections.add()` calls

**Progress events:** Emits `ai-progress` at 5/10/15/20/80/90/100 percent.

### 6.6 `converter.rs` — Mechanical template conversion (643 lines)

**Responsibilities:**
- Deterministic (non-AI) template-to-sections converter
- Two paths: WordPress live-render (fetches rendered HTML from a running WP instance) and static HTML/PHP parsing

**WordPress live-render path:**
- Fetches homepage + common slugs (about, contact, blog, sample-page) via reqwest
- Up to 4 successful HTML pages, dedup by final URL
- Splits each page into `<section>`/`<header>`/`<footer>`/`<nav>`/`<aside>` chunks
- Rewrites asset URLs to absolute

**Static HTML/PHP path:**
- Selects candidate files (WP prefers `template-parts/`, then `header/footer/single/page/index`; HTML uses all `.html` files)
- Strips PHP tags (`<?php...?>` → `<!-- php -->`)
- Splits into sections by target tags (`section`, `header`, `footer`, `nav`, `aside`)
- Tracks open/close depth to avoid nested duplicates
- Rewrites asset URLs to `assets/<path>`
- Renders `sections.js`

**Section splitting (`split_into_sections`):**
- Finds top-level target tags (`<section>`, `<header>`, `<footer>`, `<nav>`, `<aside>`)
- Tracks tag depth to avoid capturing nested duplicates
- Extracts the first `class="..."` token as the section class
- Detects `<img`/`background-image`/`<video` for `has_assets` flag
- Sorts chunks by document position

### 6.7 `folder.rs` — Folder scanning and classification (182 lines)

**Responsibilities:**
- `scan_folder` — Recursively walks a directory, skipping `node_modules`, `.git`, `vendor`, `__macosx`, `.ds_store`
- Buckets files by extension: `html_files`, `php_files`, `css_files`, `js_files`, `asset_files`
- `classify_folder` — WordPress if PHP files + `style.css` with `Theme Name:` header; HTML if any `.html` files; else unknown
- Extracts `Theme Name` and `Version` from CSS header comments

**Asset extensions:** png, jpg, jpeg, gif, svg, webp, ico, woff, woff2, ttf, eot, mp4, webm, mp3, pdf

### 6.8 `export.rs` — Zip exporter (131 lines)

**Responsibilities:**
- Creates a zip containing: `sections.js`, `blocks.js` (placeholder), `README.txt` (install instructions), screenshots, and all referenced assets (CSS, JS, images) resolved against the scan root
- Uses `zip::CompressionMethod::Deflated`

---

## 7. Frontend — Module Reference

### 7.1 `main.ts` — Dashboard (404 lines)

**Responsibilities:**
- Workspace CRUD (create, list, open, close, delete)
- Status badges (Running/Stopped) via `is_workspace_open` polling
- Auto-resume on launch (re-opens workspaces that were running when the app last quit)
- Server status display (port number)
- Tab switching (Sites / Theme Builder / Connect Agent)
- Connect Agent tab (stub — placeholder for Phase D)

**Key functions:**

| Function | Description |
|----------|-------------|
| `loadDb()` | Lazily opens SQLite via `Database.load("sqlite:webforge.db")` |
| `loadWorkspaces()` | SELECTs all workspaces, calls `renderWorkspaces` |
| `renderWorkspaces(ws)` | Builds workspace cards with status badges, open/stop/delete buttons |
| `toCamelWorkspace(ws)` | Converts snake_case DB row to camelCase for Rust `Workspace` struct |
| `openWorkspace(ws)` | Invokes `open_workspace`, updates `last_opened_at`, refreshes list |
| `closeWorkspace(ws)` | Invokes `close_workspace`, re-renders after 300ms |
| `deleteWorkspace(ws)` | Confirms, closes window, deletes folder, deletes DB rows |
| `addWorkspace()` | Validates form, generates slug, creates folder, inserts DB row |
| `autoResumeWorkspaces()` | On launch, marks running workspaces as stopped, re-opens them |
| `showServerStatus()` | Displays `Server: 127.0.0.1:<port>` |
| `initConnectAgent()` | Stub wiring for Connect Agent tab |

**Tauri commands invoked:** `is_workspace_open`, `register_site_folder`, `open_workspace`, `close_workspace`, `delete_workspace_folder`, `slugify`, `create_workspace_folder`, `get_server_port`, `get_workspaces_root`

### 7.2 `theme-builder.ts` — Theme Builder (560 lines)

**Responsibilities:**
- Full Theme Builder pipeline: scan → classify → convert (mechanical or AI) → screenshots → export
- AI settings management (provider, model, base URL, API key)
- Progress bar for AI conversion
- Screenshot generation via `html2canvas`

**Pipeline:**

```
Drop/click folder → scan_folder → classify_folder → render classification
                                                          ↓
                                    Convert (mechanical) or Convert with AI
                                                          ↓
                                    renderResults → generateScreenshots → doExport
```

**Tauri commands invoked:** `scan_folder`, `classify_folder`, `convert_template`, `convert_with_ai`, `get_provider_presets`, `load_ai_settings`, `save_ai_settings`, `render_with_screenshots`, `export_zip`

**Tauri events:** `listen("ai-progress", ...)` for AI conversion progress updates

### 7.3 `webforge-bridge.js` — Editor bridge shim (288 lines)

**Injected into `editor.html` by the HTTP server.** Monkey-patches the editor's PHP-based calls to route through Rust-native `/api/*` endpoints.

**What it overrides:**

| Original call | Redirected to | For |
|---------------|---------------|-----|
| `Webforge.Builder.saveAjax()` | `POST /api/save?site=<id>` | Local sites |
| `Webforge.Builder.saveAjax()` | `invoke("save_site", ...)` | Remote sites (legacy Tauri command) |
| `fetch("upload.php", ...)` | `POST /api/upload?site=<id>` | Local sites (FormData passthrough) |
| `fetch("upload.php", ...)` | `invoke("upload_site", ...)` | Remote sites |
| `fetch("scan.php", ...)` | `GET /api/scan?site=<id>&mediaPath=...` | Local sites |

**Other functionality:**
- Upload progress bar UI (fixed bottom, animated)
- Mtime conflict detection for local files (via `get_file_mtime` Tauri command)
- Iframe loading: sets `iframe.src` to `/site/<id>/<startUrl>` (local) or `/proxy/<startUrl>` (remote)

### 7.4 `webforge-ai-sidebar.js` — AI chat sidebar (595 lines)

**Injected into `editor.html`.** Adds a fixed-position chat sidebar on the right side of the editor.

**Features:**
- Chat interface with message history (last 8 messages sent as context)
- Settings panel (provider, base URL, API key, model) — saved to localStorage + Tauri `load_ai_settings`
- Page context gathering: selected element, page structure, available sections/components, current HTML
- AI action execution: `select_element`, `set_property` (attribute/style/text/html), `add_component`, `delete_element`, `set_html`, `undo`, `redo`
- Collapsible sidebar (toggle button)
- Calls the AI API directly via `fetch()` to `<baseUrl>/chat/completions` (not through Tauri)

**Action execution:**
- Each action records an undo mutation via `MutationObserver`
- After mutations, calls `reloadComponent()` to refresh the editor panel
- `add_component` looks up section/block/component HTML from `Webforge.SectionsGroup`/`Webforge.BlocksGroup`/`Webforge.ComponentsGroup`

---

## 8. Tauri Configuration

### `tauri.conf.json`

| Setting | Value | Notes |
|---------|-------|-------|
| `productName` | "Webforge Studio" | |
| `version` | "0.1.0" | |
| `identifier` | `com.webforge.studio` | |
| `beforeDevCommand` | `npm run dev` | Starts Vite dev server |
| `devUrl` | `http://localhost:1420` | Where the webview loads in dev mode |
| `beforeBuildCommand` | `npm run build` | Builds frontend before Rust compilation |
| `frontendDist` | `../dist` | Embedded frontend assets (relative to `src-tauri/`) |
| `withGlobalTauri` | `true` | Exposes `window.__TAURI__` for non-module scripts |
| Window size | 1200×800 (min 900×600) | Main dashboard window |
| `csp` | `null` | No Content Security Policy (permissive) |
| `sql.preload` | `["sqlite:webforge.db"]` | Auto-loads the database on app start |
| `bundle.resources` | `["resources/webforge/**/*"]` | Bundles the editor into the app |
| `bundle.targets` | `"all"` | Builds DMG (macOS) + NSIS/MSI (Windows) |

### `.cargo/config.toml`

Sets `-mmacosx-version-min=12.0` for both `aarch64-apple-darwin` and `x86_64-apple-darwin` targets, ensuring binaries run on macOS 12 (Monterey) and later.

### `capabilities/default.json`

Permissions granted to the main window:
- `core:default`, `core:window:default`, `core:webview:default`
- `opener:default`
- `sql:default`, `sql:allow-execute`, `sql:allow-load`, `sql:allow-select`, `sql:allow-close`
- `fs:default`, `fs:allow-read-file`, `fs:allow-read-dir`, `fs:allow-write-file`
- `dialog:default`, `dialog:allow-save`, `dialog:allow-open`

---

## 9. Embedded HTTP Server — Route Reference

### Static file serving

| Route | Source | Notes |
|-------|--------|-------|
| `GET /` | `resources/webforge/editor.html` | Root defaults to editor |
| `GET /<path>` | `resources/webforge/<path>` | Static assets (CSS, JS, images) |
| `GET /editor.html` | Dynamic | Injects `defaultPages` + bridge shim + AI sidebar |
| `GET /site/<id>/<path>` | Workspace folder | Serves files from the registered workspace |
| `GET /site/<id>/` | Workspace folder | Defaults to `index.html` for directories |

### API routes (PHP replacement)

| Route | Method | Body | Returns |
|-------|--------|------|---------|
| `/api/save?site=<id>` | POST | `application/x-www-form-urlencoded`: `file`, `html`, `startTemplateUrl?` | `text/plain`: "File saved '<path>'" |
| `/api/save?site=<id>&action=rename` | POST | `file`, `newfile` | `text/plain`: "File renamed" |
| `/api/save?site=<id>&action=delete` | POST | `file` | `text/plain`: "File deleted" |
| `/api/save?site=<id>&action=saveReusable` | POST | `type`, `name`, `html` | `text/plain`: "File saved" |
| `/api/save?site=<id>&action=oembedProxy` | POST | `url` | `application/json`: oembed response |
| `/api/upload?site=<id>` | POST | `multipart/form-data`: `file`, `mediaPath?` | `text/plain`: "<mediaPath>/<filename>" |
| `/api/scan?site=<id>&mediaPath=<path>` | GET | — | `application/json`: nested file tree |
| `/api/pages?site=<id>` | GET | — | `application/json`: page list array |

### Proxy route

| Route | Method | Notes |
|-------|--------|-------|
| `/proxy/<percent-encoded-url>` | GET | Fetches remote URL, rewrites asset URLs to `/proxy/`, injects bridge shim |

---

## 10. Theme Builder Pipeline

```
User drops/selects a folder
        │
        ▼
  scan_folder (Rust)          → FolderScan { root, entries, html_files, php_files, ... }
        │
        ▼
  classify_folder (Rust)      → Classification { kind: "wordpress"|"html"|"unknown", theme_name, ... }
        │
        ▼
  ┌─────────────────────────────────────────────────┐
  │  User chooses:                                   │
  │  ├── Convert (mechanical) → convert_template     │
  │  └── Convert with AI      → convert_with_ai      │
  └─────────────────────────────────────────────────┘
        │
        ▼
  ConversionResult { sections, sections_js, asset_paths, warnings }
        │
        ▼
  renderResults (TS)          → Section cards with key, name, tag, HTML preview
        │
        ▼
  generateScreenshots (TS)    → html2canvas per section → PNG bytes
        │                        → render_with_screenshots (Rust) updates sections.js
        ▼
  doExport (TS)               → export_zip (Rust) → .zip with sections.js + assets + screenshots
```

**Output zip structure:**
```
export.zip
├── sections.js              # Webforge.SectionsGroup + Sections.add() calls
├── blocks.js                # Placeholder (empty BlocksGroup)
├── README.txt               # Install instructions
├── screenshots/             # PNG thumbnails per section
│   ├── hero.png
│   └── features.png
└── assets/                  # All referenced CSS, JS, images
    ├── style.css
    └── img/logo.png
```

---

## 11. AI Integration

### Two AI surfaces

| Surface | Where | API | Purpose |
|---------|-------|-----|---------|
| Theme Builder AI conversion | `ai_agent.rs` (Rust) | OpenAI-compatible `/v1/chat/completions` | Converts templates to sections in one shot |
| Editor AI sidebar | `webforge-ai-sidebar.js` (JS) | Same API, called directly via `fetch()` | Interactive chat assistant for editing the live page |

### Settings storage

- **Rust side:** `~/.config/com.webforge.studio/ai-settings.json` (JSON file with `provider`, `baseUrl`, `apiKey`, `model`)
- **JS side:** `localStorage["webforge-ai-settings"]` (mirror, used when Tauri is unavailable)
- The sidebar loads from Tauri first, falls back to localStorage

### AI sidebar action protocol

The AI returns JSON:
```json
{
  "thinking": "I'll add a hero section...",
  "actions": [
    { "tool": "select_element", "args": { "selector": ".hero" } },
    { "tool": "set_property", "args": { "property_type": "text", "value": "Welcome!" } },
    { "tool": "add_component", "args": { "kind": "section", "component_type": "hero-1" } }
  ],
  "message": "Added a hero section and updated the heading."
}
```

**Supported tools:** `select_element`, `set_property` (attribute/style/text/html), `add_component`, `delete_element`, `set_html`, `undo`, `redo`

---

## 12. Build & CI

### Local development

```bash
# Install deps
npm install

# Run in dev mode (Vite + Tauri hot reload)
npx tauri dev

# Build production binary (DMG on macOS, EXE on Windows)
npx tauri build

# Run Rust tests
cd src-tauri && cargo test --lib

# TypeScript typecheck
npx tsc --noEmit
```

### CI workflow (`ci.yml`)

Triggers: push to `main`/`master`, pull requests.

| Step | macOS 14 (ARM) | Windows |
|------|----------------|---------|
| Checkout | ✅ | ✅ |
| Install Node 20 | ✅ | ✅ |
| Install Rust + target | ✅ | ✅ |
| Cache Rust crates | ✅ | ✅ |
| `npm ci` | ✅ | ✅ |
| `npx tsc --noEmit` | ✅ | ✅ |
| `cargo test --lib` | ✅ | ✅ |
| `cargo build --release --target` | ✅ | ✅ |

### Release workflow (`release.yml`)

Triggers: `v*` tag push, `workflow_dispatch`.

| Target | Runner | Method | Output |
|--------|--------|--------|--------|
| macOS ARM | macos-14 | Native | `Webforge.Studio_0.1.0_aarch64.dmg` |
| macOS Intel | macos-14 | Cross-compile (`x86_64-apple-darwin`) | `Webforge.Studio_0.1.0_x64.dmg` |
| Windows | windows-latest | Native | `Webforge.Studio_0.1.0_x64-setup.exe` + `.msi` |

**macOS deployment target:** `MACOSX_DEPLOYMENT_TARGET=12.0` + `-mmacosx-version-min=12.0` → binaries run on macOS 12+

**Signing:** Currently unsigned (Gatekeeper warning on macOS, SmartScreen on Windows).

---

## 13. File Ownership & Disk Layout

### App bundle (macOS)

```
Webforge Studio.app/
├── Contents/
│   ├── MacOS/
│   │   └── webforge-studio          # The Rust binary
│   ├── Resources/
│   │   ├── icon.icns
│   │   └── resources/
│   │       └── webforge/            # The bundled editor (~74 MB)
│   │           ├── editor.html
│   │           ├── libs/
│   │           ├── demo/
│   │           ├── webforge-bridge.js
│   │           ├── webforge-ai-sidebar.js
│   │           └── ...
│   └── Info.plist
```

### User data

| Path | Contents |
|------|----------|
| `~/Library/Application Support/com.webforge.studio/webforge.db` | SQLite database (macOS) |
| `%APPDATA%/com.webforge.studio/webforge.db` | SQLite database (Windows) |
| `~/.config/com.webforge.studio/ai-settings.json` | AI provider settings |
| `~/Webforge Studio Workspaces/<slug>/` | Managed workspace folders |
| `~/Webforge Studio Workspaces/<slug>/index.html` | Starter page (created on workspace creation) |
| `~/Webforge Studio Workspaces/<slug>/uploads/` | Uploaded media files |

---

## 14. Known Issues & TODOs

### Bugs to fix

| Issue | Severity | Status |
|-------|----------|--------|
| Installers crash on launch (stale dist + no signing) | High | Pending |
| Debug code in lib.rs (eval error trap, [SETUP] prints) | Medium | Pending |
| Dead code in openWorkspace() (register_site_folder condition never true) | Low | Pending |
| Legacy open_site/register_site_folder commands coexist with workspace commands | Low | Pending |

### E2E tests needed

| Test | Priority |
|------|----------|
| Create workspace → open editor → save → verify on disk | High |
| Upload image in editor → verify in workspace folder | High |
| Media modal scan shows files | High |
| File manager shows pages (defaultPages injection) | High |
| Two workspaces open simultaneously | High |
| Theme Builder works after workspace refactor | Medium |
| AI sidebar loads and functions | Medium |

### Features incomplete

| Phase | Description | Priority |
|-------|-------------|----------|
| C | Rust-native MCP server (12 tools, mcp subcommand) | High |
| D | Connect Agent tab UI (real MCP control) | High |
| E | Production hardening (graceful shutdown, logs, DMG signing) | Medium |
| E | Windows EXE verification on actual Windows | Medium |

---

## 15. Glossary

| Term | Meaning |
|------|---------|
| **Webforge** | The whitelabelled fork of VVVEB.js (the visual page builder) |
| **Workspace** | A site project managed by Webforge Studio — has a folder, editor window, and DB row |
| **Section** | A reusable HTML block (e.g., hero, features, footer) that can be dragged into a page |
| **SectionsGroup** | A named collection of sections in the Webforge editor (e.g., "imported", "landing") |
| **Bridge shim** | `webforge-bridge.js` — injected into editor.html to reroute PHP calls to Rust `/api/*` |
| **AI sidebar** | `webforge-ai-sidebar.js` — injected chat assistant in the editor |
| **Managed folder** | A workspace folder created and owned by Webforge Studio under `~/Webforge Studio Workspaces/` |
| **Imported folder** | An existing folder the user pointed at; not deleted on workspace deletion |
| **Remote site** | A workspace that forwards saves to a remote server's `save.php` |
| **MCP** | Model Context Protocol — lets external AI clients (Claude Desktop, Cursor) drive the editor |
| **defaultPages** | A JS object injected into editor.html listing all available pages for the file manager |
| **@@include** | SSI-style template directive used in VVVEB.js source templates (resolved at build time) |

---

*This document was generated from a full audit of the codebase on June 26, 2026.*