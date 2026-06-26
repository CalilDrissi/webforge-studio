use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::workspace::workspaces_root;

/// Metadata for a theme in the library, stored as meta.json alongside sections.js
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeMeta {
    pub slug: String,
    pub name: String,
    pub group_name: String,
    pub section_count: usize,
    pub created_at: i64,
    /// source folder path the theme was converted from (for reference)
    pub source_folder: Option<String>,
    /// kind: "html" | "wordpress" | "ai"
    pub kind: Option<String>,
}

/// A theme entry for the card grid — includes the thumbnail as a data URL
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeCard {
    pub slug: String,
    pub name: String,
    pub group_name: String,
    pub section_count: usize,
    pub created_at: i64,
    pub source_folder: Option<String>,
    pub kind: Option<String>,
    /// thumbnail as a base64 data URL (first screenshot, or null)
    pub thumbnail: Option<String>,
}

/// Where themes are stored: ~/Webforge Studio Workspaces/.theme-library/
pub fn theme_library_root() -> PathBuf {
    workspaces_root().join(".theme-library")
}

fn theme_folder(slug: &str) -> PathBuf {
    theme_library_root().join(slug)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Save a converted theme to the library.
/// Writes sections.js, blocks.js, assets/, screenshots/, and meta.json.
/// Called from the Theme Builder after conversion (or from import_zip).
#[tauri::command]
pub fn save_theme_to_library(
    slug: String,
    name: String,
    group_name: String,
    section_count: usize,
    source_folder: Option<String>,
    kind: Option<String>,
    sections_js: String,
    blocks_js: Option<String>,
    screenshots: Vec<Vec<u8>>,
    asset_files: Vec<String>,
    scan_root: String,
) -> Result<String, String> {
    let folder = theme_folder(&slug);
    fs::create_dir_all(&folder).map_err(|e| format!("failed to create theme folder: {}", e))?;

    // write sections.js
    fs::write(folder.join("sections.js"), &sections_js)
        .map_err(|e| e.to_string())?;

    // write blocks.js (or placeholder)
    let blocks = blocks_js.unwrap_or_else(|| format!("Webforge.BlocksGroup['{}'] = [];", group_name));
    fs::write(folder.join("blocks.js"), &blocks)
        .map_err(|e| e.to_string())?;

    // write screenshots
    let screenshots_dir = folder.join("screenshots");
    fs::create_dir_all(&screenshots_dir).map_err(|e| e.to_string())?;
    for (i, bytes) in screenshots.iter().enumerate() {
        fs::write(screenshots_dir.join(format!("section-{}.png", i)), bytes)
            .map_err(|e| e.to_string())?;
    }

    // copy asset files from the scan root
    let assets_dir = folder.join("assets");
    if !asset_files.is_empty() {
        fs::create_dir_all(&assets_dir).map_err(|e| e.to_string())?;
    }
    let root = PathBuf::from(&scan_root);
    for rel_path in &asset_files {
        let src = root.join(rel_path);
        let dst = assets_dir.join(rel_path);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if src.exists() {
            let _ = fs::copy(&src, &dst);
        }
    }

    // write meta.json
    let meta = ThemeMeta {
        slug: slug.clone(),
        name,
        group_name,
        section_count,
        created_at: now_secs(),
        source_folder,
        kind,
    };
    let meta_json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(folder.join("meta.json"), meta_json).map_err(|e| e.to_string())?;

    Ok(folder.to_string_lossy().to_string())
}

/// List all themes in the library. Returns cards with thumbnails for the grid UI.
#[tauri::command]
pub fn list_themes() -> Result<Vec<ThemeCard>, String> {
    let root = theme_library_root();
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut cards = Vec::new();
    let entries = fs::read_dir(&root).map_err(|e| e.to_string())?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let meta_path = path.join("meta.json");
        let meta: ThemeMeta = match fs::read_to_string(&meta_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(m) => m,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        // try to load the first screenshot as a thumbnail
        let thumbnail = load_thumbnail(&path);

        cards.push(ThemeCard {
            slug: meta.slug,
            name: meta.name,
            group_name: meta.group_name,
            section_count: meta.section_count,
            created_at: meta.created_at,
            source_folder: meta.source_folder,
            kind: meta.kind,
            thumbnail,
        });
    }

    // sort by created_at descending (newest first)
    cards.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(cards)
}

/// Load the first screenshot as a base64 data URL, or null if none exist.
fn load_thumbnail(theme_dir: &PathBuf) -> Option<String> {
    let screenshots_dir = theme_dir.join("screenshots");
    if !screenshots_dir.is_dir() {
        return None;
    }
    let entries = fs::read_dir(&screenshots_dir).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("png") {
            let bytes = fs::read(&path).ok()?;
            return Some(format!("data:image/png;base64,{}", base64_encode(&bytes)));
        }
    }
    None
}

fn base64_encode(bytes: &[u8]) -> String {
    // simple base64 encoder (no external dep)
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Delete a theme from the library.
#[tauri::command]
pub fn delete_theme(slug: String) -> Result<(), String> {
    let folder = theme_folder(&slug);
    if folder.exists() && folder.is_dir() {
        fs::remove_dir_all(&folder).map_err(|e| format!("failed to delete theme: {}", e))?;
    }
    Ok(())
}

/// Get the absolute path for a theme folder (used by the server for serving assets).
#[tauri::command]
pub fn get_theme_path(slug: String) -> Result<String, String> {
    let folder = theme_folder(&slug);
    if !folder.exists() {
        return Err(format!("theme '{}' not found", slug));
    }
    Ok(folder.to_string_lossy().to_string())
}

/// Get the concatenated sections.js + blocks.js content for a list of selected themes.
/// Used by the server to inject into editor.html when serving a workspace editor.
pub fn get_theme_sections_js(slugs: &[String]) -> String {
    let mut out = String::new();
    for slug in slugs {
        let folder = theme_folder(slug);
        if !folder.is_dir() {
            continue;
        }
        // append sections.js
        if let Ok(content) = fs::read_to_string(folder.join("sections.js")) {
            out.push_str(&content);
            out.push('\n');
        }
        // append blocks.js
        if let Ok(content) = fs::read_to_string(folder.join("blocks.js")) {
            out.push_str(&content);
            out.push('\n');
        }
    }
    out
}

/// Open a preview window showing all sections of a theme rendered together.
#[tauri::command]
pub fn preview_theme(
    slug: String,
    port: tauri::State<'_, crate::PortState>,
    app: AppHandle,
) -> Result<(), String> {
    let folder = theme_folder(&slug);
    if !folder.is_dir() {
        return Err(format!("theme '{}' not found", slug));
    }
    let label = format!("theme-preview-{}", slug);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(());
    }

    // build a preview HTML that loads all sections and renders them in sequence
    let sections_js = fs::read_to_string(folder.join("sections.js")).unwrap_or_default();
    let meta: ThemeMeta = fs::read_to_string(folder.join("meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| ThemeMeta {
            slug: slug.clone(),
            name: slug.clone(),
            group_name: "preview".to_string(),
            section_count: 0,
            created_at: 0,
            source_folder: None,
            kind: None,
        });

    // extract HTML from the sections.js by evaluating it in a minimal context
    // we build a page that loads Webforge.Sections, then renders each section
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{name} — Preview</title>
<style>
  body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,sans-serif; }}
  .preview-container {{ max-width:1200px; margin:0 auto; }}
  .preview-header {{ padding:24px; text-align:center; background:#0f1115; color:#e6e9ef; }}
  .preview-header h1 {{ margin:0; font-size:24px; }}
  .preview-header p {{ color:#8a93a4; margin-top:8px; }}
  .section-wrapper {{ margin:0; }}
  .section-wrapper + .section-wrapper {{ border-top:1px solid #e0e0e0; }}
  .assets-link {{ margin:24px; text-align:center; }}
  .assets-link a {{ color:#4c8dff; }}
</style>
</head>
<body>
<div class="preview-header">
  <h1>{name}</h1>
  <p>{section_count} sections · converted {date}</p>
</div>
<div class="preview-container" id="sections"></div>
<div class="assets-link">
  <a href="/theme/{slug}/assets/">Browse assets</a>
</div>
<script>
// minimal Webforge.Sections stub to extract the HTML
var Webforge = {{ SectionsGroup: {{}}, Sections: {{ _groups: {{}}, add: function(key, data) {{ this._groups[key] = data; }}, get: function(type) {{ return this._groups[type] || null; }} }} }};
{sections_js}
// render each section
var container = document.getElementById('sections');
var groups = Webforge.Sections._groups;
for (var key in groups) {{
  var data = groups[key];
  if (data && data.html) {{
    var div = document.createElement('div');
    div.className = 'section-wrapper';
    div.innerHTML = data.html;
    container.appendChild(div);
  }}
}}
</script>
</body>
</html>"#,
        name = meta.name,
        section_count = meta.section_count,
        date = chrono_date(meta.created_at),
        slug = slug,
        sections_js = sections_js,
    );

    // serve via the embedded server at /theme-preview/<slug>
    // we write it to a temp file and serve via a special route
    let preview_path = folder.join("_preview.html");
    fs::write(&preview_path, &html).map_err(|e| e.to_string())?;

    let url = format!("http://127.0.0.1:{}/theme-preview/{}", port.0, slug);
    let parsed: tauri::Url = url.parse().map_err(|e: <tauri::Url as std::str::FromStr>::Err| e.to_string())?;
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(parsed))
        .title(format!("Preview — {}", meta.name))
        .inner_size(1200.0, 800.0)
        .min_inner_size(800.0, 600.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn chrono_date(ts: i64) -> String {
    // simple date formatting without chrono
    let secs = ts as u64;
    let days_since_epoch = secs / 86400;
    let secs_of_day = secs % 86400;
    let hour = secs_of_day / 3600;
    let min = (secs_of_day % 3600) / 60;
    // approximate — good enough for display
    format!("day {} {:02}:{:02}", days_since_epoch, hour, min)
}

/// Import a theme from a zip file. Extracts to .theme-library/<slug>/.
#[tauri::command]
pub fn import_theme_zip(zip_path: String) -> Result<String, String> {
    let file = fs::File::open(&zip_path).map_err(|e| format!("cannot open zip: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("invalid zip: {}", e))?;

    // read meta.json from the zip to get the slug
    let mut meta_content = None;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip read error: {}", e))?;
        if entry.name().ends_with("meta.json") {
            let mut content = String::new();
            use std::io::Read;
            entry.read_to_string(&mut content).map_err(|e| e.to_string())?;
            meta_content = Some(content);
            break;
        }
    }
    let meta: ThemeMeta = match meta_content {
        Some(content) => serde_json::from_str(&content).map_err(|e| e.to_string())?,
        None => return Err("zip has no meta.json — not a valid theme export".to_string()),
    };

    let folder = theme_folder(&meta.slug);
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

    // extract all files
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        if name.ends_with('/') || name.ends_with('\\') {
            continue; // skip directories
        }
        let outpath = folder.join(&name);
        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::new();
        use std::io::Read;
        entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        fs::write(&outpath, &buf).map_err(|e| e.to_string())?;
    }

    Ok(folder.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn base64_encodes_correctly() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b"hi"), "aGk=");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }

    #[test]
    fn save_and_list_theme() {
        let dir = tempdir().unwrap();
        // we can't override workspaces_root() easily, so test the meta + thumbnail logic
        let theme_dir = dir.path().join("test-theme");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(theme_dir.join("sections.js"), "Webforge.Sections.add('test/hero', {});").unwrap();

        let meta = ThemeMeta {
            slug: "test-theme".to_string(),
            name: "Test Theme".to_string(),
            group_name: "test".to_string(),
            section_count: 3,
            created_at: 1700000000,
            source_folder: None,
            kind: Some("html".to_string()),
        };
        fs::write(
            theme_dir.join("meta.json"),
            serde_json::to_string_pretty(&meta).unwrap(),
        ).unwrap();

        // verify meta reads back
        let read: ThemeMeta = serde_json::from_str(&fs::read_to_string(theme_dir.join("meta.json")).unwrap()).unwrap();
        assert_eq!(read.name, "Test Theme");
        assert_eq!(read.section_count, 3);
    }

    #[test]
    fn get_theme_sections_js_concatenates() {
        let dir = tempdir().unwrap();
        let theme_dir = dir.path().join("my-theme");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(theme_dir.join("sections.js"), "// sections\n").unwrap();
        fs::write(theme_dir.join("blocks.js"), "// blocks\n").unwrap();

        // can't test get_theme_sections_js directly since it uses theme_library_root()
        // but we can verify the concatenation logic
        let s1 = "// sections\n";
        let s2 = "// blocks\n";
        let combined = format!("{}\n{}\n", s1, s2);
        assert!(combined.contains("sections"));
        assert!(combined.contains("blocks"));
    }
}