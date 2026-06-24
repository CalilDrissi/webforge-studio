use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub path: String,
    pub relative_path: String,
    pub is_dir: bool,
    pub size: u64,
    pub ext: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FolderScan {
    pub root: String,
    pub entries: Vec<FileEntry>,
    pub html_files: Vec<FileEntry>,
    pub php_files: Vec<FileEntry>,
    pub css_files: Vec<FileEntry>,
    pub js_files: Vec<FileEntry>,
    pub asset_files: Vec<FileEntry>,
}

#[tauri::command]
pub fn scan_folder(folder_path: String) -> Result<FolderScan, String> {
    let root = Path::new(&folder_path);
    if !root.is_dir() {
        return Err(format!("Not a directory: {}", folder_path));
    }
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    let mut entries: Vec<FileEntry> = Vec::new();
    walk_dir(&canonical, &canonical, &mut entries)?;

    let mut html_files = Vec::new();
    let mut php_files = Vec::new();
    let mut css_files = Vec::new();
    let mut js_files = Vec::new();
    let mut asset_files = Vec::new();

    for e in &entries {
        if e.is_dir {
            continue;
        }
        match e.ext.as_str() {
            "html" | "htm" => html_files.push(e.clone()),
            "php" => php_files.push(e.clone()),
            "css" => css_files.push(e.clone()),
            "js" => js_files.push(e.clone()),
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "woff"
            | "woff2" | "ttf" | "eot" | "mp4" | "webm" | "mp3" | "pdf" => {
                asset_files.push(e.clone())
            }
            _ => {}
        }
    }

    Ok(FolderScan {
        root: canonical.to_string_lossy().to_string(),
        entries,
        html_files,
        php_files,
        css_files,
        js_files,
        asset_files,
    })
}

fn walk_dir(dir: &Path, root: &Path, out: &mut Vec<FileEntry>) -> Result<(), String> {
    let rd = fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in rd {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        out.push(FileEntry {
            path: path.to_string_lossy().to_string(),
            relative_path: relative,
            is_dir: meta.is_dir(),
            size: meta.len(),
            ext,
        });
        if meta.is_dir() {
            // skip common junk dirs
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if ["node_modules", ".git", "vendor", "__macosx", ".ds_store"].iter().any(|s| name.eq_ignore_ascii_case(s)) {
                continue;
            }
            walk_dir(&path, root, out)?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Classification {
    pub kind: String,
    pub theme_name: Option<String>,
    pub theme_version: Option<String>,
    pub template_count: usize,
    pub notes: Vec<String>,
}

#[tauri::command]
pub fn classify_folder(scan: FolderScan) -> Result<Classification, String> {
    let has_php = !scan.php_files.is_empty();
    let style_css = scan.css_files.iter().find(|f| {
        Path::new(&f.relative_path)
            .file_name()
            .map(|n| n.eq_ignore_ascii_case("style.css"))
            .unwrap_or(false)
    });

    let is_wp = has_php
        && style_css.is_some()
        && {
            let content = fs::read_to_string(&style_css.unwrap().path).map_err(|e| e.to_string())?;
            content.contains("Theme Name:")
        };

    if is_wp {
        let content = fs::read_to_string(&style_css.unwrap().path).map_err(|e| e.to_string())?;
        let theme_name = extract_header_field(&content, "Theme Name");
        let theme_version = extract_header_field(&content, "Version");
        Ok(Classification {
            kind: "wordpress".to_string(),
            theme_name,
            theme_version,
            template_count: scan.php_files.len(),
            notes: vec![
                format!("Detected WP theme with {} PHP templates", scan.php_files.len()),
                "PHP tags will be stripped and replaced with placeholders".to_string(),
                "template-parts/ will be treated as sections".to_string(),
            ],
        })
    } else if !scan.html_files.is_empty() {
        Ok(Classification {
            kind: "html".to_string(),
            theme_name: None,
            theme_version: None,
            template_count: scan.html_files.len(),
            notes: vec![
                format!("Detected static HTML with {} files", scan.html_files.len()),
                "Each <section>/<header>/<footer>/<nav> becomes a Webforge section".to_string(),
            ],
        })
    } else {
        Ok(Classification {
            kind: "unknown".to_string(),
            theme_name: None,
            theme_version: None,
            template_count: 0,
            notes: vec!["No .html or .php files found".to_string()],
        })
    }
}

fn extract_header_field(css: &str, field: &str) -> Option<String> {
    for line in css.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&format!("{}:", field)) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

pub fn read_file(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| e.to_string())
}

pub fn read_file_bytes(path: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| e.to_string())
}