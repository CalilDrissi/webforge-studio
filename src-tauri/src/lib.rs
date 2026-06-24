use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_sql::{Builder as SqlBuilder, Migration, MigrationKind};
use tokio::sync::RwLock;

mod ai_agent;
mod converter;
mod export;
mod folder;
mod server;
mod site_io;
mod workspace;

use server::start_server;
use site_io::{get_file_mtime, save_site, upload_site};

/// Shared map of site_id -> local folder path, used by both the embedded HTTP server
/// (to resolve /site/<id>/... requests) and the open_site/register_site_folder commands.
pub type SiteFolders = Arc<RwLock<HashMap<i64, PathBuf>>>;

pub struct PortState(pub u16);
pub struct SiteFoldersState(pub SiteFolders);

#[tauri::command]
fn open_site(
    site_id: i64,
    name: String,
    kind: String,
    start_url: String,
    folder_path: Option<String>,
    server_url: Option<String>,
    port: tauri::State<PortState>,
    site_folders: tauri::State<SiteFoldersState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // for local sites, register the folder with the embedded server so /site/<id>/... resolves
    if kind == "local" {
        if let Some(folder) = folder_path.clone() {
            let canonical =
                std::fs::canonicalize(&folder).unwrap_or_else(|_| PathBuf::from(&folder));
            // use blocking write since site_folders is a tokio RwLock but we're in a sync command
            // (tauri commands run on a thread where blocking is acceptable as long as we don't
            // hold it long; the runtime is separate)
            let folders = site_folders.0.clone();
            // spawn a tiny blocking task to write into the tokio RwLock
            tauri::async_runtime::block_on(async {
                let mut g = folders.write().await;
                g.insert(site_id, canonical);
            });
        }
    }

    // unused for now - server_url is passed to save_site/upload_site via the frontend bridge
    // by encoding folder_path/server_url into the editor URL query string
    let folder_encoded = folder_path
        .as_ref()
        .map(|p| urlencoding::encode(p).to_string())
        .unwrap_or_default();
    let server_encoded = server_url
        .as_ref()
        .map(|u| urlencoding::encode(u).to_string())
        .unwrap_or_default();

    let editor_url = format!(
        "http://127.0.0.1:{}/editor.html?site={}&kind={}&startUrl={}&folder={}&server={}",
        port.0,
        site_id,
        kind,
        urlencoding::encode(&start_url),
        folder_encoded,
        server_encoded
    );

    let label = format!("site-{}", site_id);
    let parsed_url: tauri::Url = editor_url
        .parse()
        .map_err(|e: <tauri::Url as std::str::FromStr>::Err| e.to_string())?;
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(parsed_url))
        .title(format!("Webforge Studio - {}", name))
        .inner_size(1400.0, 900.0)
        .min_inner_size(900.0, 600.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_server_port(port: tauri::State<PortState>) -> u16 {
    port.0
}

#[tauri::command]
fn register_site_folder(
    site_id: i64,
    folder_path: String,
    site_folders: tauri::State<SiteFoldersState>,
) -> Result<(), String> {
    let canonical =
        std::fs::canonicalize(&folder_path).unwrap_or_else(|_| PathBuf::from(&folder_path));
    let folders = site_folders.0.clone();
    tauri::async_runtime::block_on(async {
        let mut g = folders.write().await;
        g.insert(site_id, canonical);
    });
    Ok(())
}

fn site_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "create_sites_table",
            sql: "CREATE TABLE IF NOT EXISTS sites (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                url TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                last_opened_at INTEGER
            );",
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "add_kind_and_paths",
            sql: "ALTER TABLE sites ADD COLUMN kind TEXT NOT NULL DEFAULT 'remote';
                  ALTER TABLE sites ADD COLUMN folder_path TEXT;
                  ALTER TABLE sites ADD COLUMN server_url TEXT;
                  ALTER TABLE sites ADD COLUMN start_page TEXT;",
            kind: MigrationKind::Up,
        },
        Migration {
            version: 3,
            description: "create_workspaces_tables",
            sql: "CREATE TABLE IF NOT EXISTS workspaces (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    slug TEXT NOT NULL UNIQUE,
                    kind TEXT NOT NULL DEFAULT 'local-folder',
                    folder_path TEXT NOT NULL,
                    start_page TEXT NOT NULL DEFAULT 'index.html',
                    server_url TEXT,
                    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                    last_opened_at INTEGER
                  );
                  CREATE TABLE IF NOT EXISTS workspace_prefs (
                    workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    key TEXT NOT NULL,
                    value TEXT NOT NULL,
                    PRIMARY KEY (workspace_id, key)
                  );
                  CREATE TABLE IF NOT EXISTS workspace_runtime (
                    workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    status TEXT NOT NULL DEFAULT 'stopped',
                    window_label TEXT,
                    port INTEGER,
                    updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                    PRIMARY KEY (workspace_id)
                  );
                  CREATE INDEX IF NOT EXISTS idx_workspaces_last_opened ON workspaces(last_opened_at);
                  CREATE INDEX IF NOT EXISTS idx_workspaces_slug ON workspaces(slug);",
            kind: MigrationKind::Up,
        },
        Migration {
            version: 4,
            description: "migrate_sites_to_workspaces",
            sql: "INSERT OR IGNORE INTO workspaces (name, slug, kind, folder_path, start_page, server_url, created_at, last_opened_at)
                  SELECT
                    name,
                    lower(replace(replace(replace(replace(name, ' ', '-'), '/', '-'), '\\\\', '-'), '.', '-')) || '-' || id,
                    CASE WHEN kind = 'local' THEN 'local-import' ELSE 'remote' END,
                    COALESCE(folder_path, 'unused-' || id),
                    COALESCE(start_page, 'index.html'),
                    server_url,
                    created_at,
                    last_opened_at
                  FROM sites
                  WHERE NOT EXISTS (SELECT 1 FROM workspaces WHERE workspaces.name = sites.name);",
            kind: MigrationKind::Up,
        },
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let site_folders: SiteFolders = Arc::new(RwLock::new(HashMap::new()));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            SqlBuilder::default()
                .add_migrations("sqlite:webforge.db", site_migrations())
                .build(),
        )
        .manage(SiteFoldersState(site_folders.clone()))
        .setup(move |app| {
            // resolve the bundled webforge resources directory
            let resource_dir = app
                .path()
                .resource_dir()
                .map_err(|e| e.to_string())?
                .join("webforge");
            let resource_dir = if !resource_dir.exists() {
                // dev mode: fall back to the source resources dir
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/webforge")
            } else {
                resource_dir
            };

            // start the embedded http server, sharing the same site_folders map
            let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
            let state = rt
                .block_on(start_server(resource_dir, site_folders.clone()))
                .map_err(|e| e.to_string())?;

            app.manage(PortState(state.port));
            // leak the runtime to keep the server alive for the app lifetime
            std::mem::forget(rt);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_site,
            get_server_port,
            register_site_folder,
            save_site,
            upload_site,
            get_file_mtime,
            folder::scan_folder,
            folder::classify_folder,
            converter::convert_template,
            converter::render_with_screenshots,
            export::export_zip,
            ai_agent::convert_with_ai,
            ai_agent::get_provider_presets,
            ai_agent::save_ai_settings,
            ai_agent::load_ai_settings,
            workspace::slugify,
            workspace::create_workspace_folder,
            workspace::open_workspace,
            workspace::close_workspace,
            workspace::is_workspace_open,
            workspace::delete_workspace_folder,
            workspace::get_workspaces_root,
            workspace::get_workspace_size
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}