use crate::folder::{read_file, read_file_bytes, FolderScan};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use zip::ZipWriter;

#[derive(Debug, Serialize, Deserialize)]
pub struct ScreenshotEntry {
    pub key: String,
    pub filename: String, // e.g. "hero-1.png"
    pub bytes: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportInput {
    pub scan: FolderScan,
    pub sections_js: String,
    pub group_name: String,
    pub asset_paths: Vec<String>,
    pub output_path: String,
    /// PNG screenshots to include under screenshots/
    #[serde(default)]
    pub screenshots: Vec<ScreenshotEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportResult {
    pub path: String,
    pub bytes_written: u64,
    pub file_count: usize,
}

#[tauri::command]
pub fn export_zip(input: ExportInput) -> Result<ExportResult, String> {
    let out_path = Path::new(&input.output_path);
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = fs::File::create(out_path).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);

    let opts: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // 1. sections.js at the root (already has image: fields pointing to screenshots/)
    zip.start_file("sections.js", opts).map_err(|e| e.to_string())?;
    zip.write_all(input.sections_js.as_bytes()).map_err(|e| e.to_string())?;

    // 2. blocks.js placeholder
    let blocks_js = format!(
        "/* Webforge Studio - generated blocks (none) */\nWebforge.BlocksGroup['{}'] = [];\n",
        input.group_name
    );
    zip.start_file("blocks.js", opts).map_err(|e| e.to_string())?;
    zip.write_all(blocks_js.as_bytes()).map_err(|e| e.to_string())?;

    // 3. README
    let has_screenshots = !input.screenshots.is_empty();
    let readme = format!(
        "Webforge Studio export\n=======================\n\n\
Group: {group}\nFiles: sections.js, blocks.js, assets/, {screenshots_dir}\n\n\
Install:\n\
1. Copy sections.js, blocks.js, screenshots/ and assets/ into your Webforge project\n\
   (e.g. into demo/landing/sections/ or wherever your sections live).\n\
2. Add <script src=\"sections.js\"></script> and <script src=\"blocks.js\"></script>\n\
   to editor.html after the existing sections/blocks scripts.\n\
3. Copy the assets/ folder to your web root so the relative paths resolve.\n\
4. Reload the editor - the new sections appear in the Sections panel under the\n\
   '{group}' group with thumbnail previews.\n",
        group = input.group_name,
        screenshots_dir = if has_screenshots { "screenshots/" } else { "(no screenshots)" }
    );
    zip.start_file("README.txt", opts).map_err(|e| e.to_string())?;
    zip.write_all(readme.as_bytes()).map_err(|e| e.to_string())?;

    let mut file_count = 3;

    // 4. screenshots
    for s in &input.screenshots {
        let zip_path = format!("screenshots/{}", s.filename);
        if zip.start_file(&zip_path, opts).is_err() {
            continue;
        }
        let _ = zip.write_all(&s.bytes);
        file_count += 1;
    }

    // 5. assets referenced by the sections
    for asset_rel in &input.asset_paths {
        let cleaned = asset_rel.trim_start_matches("./").trim_start_matches('/');
        let candidate = Path::new(&input.scan.root).join(cleaned);
        if !candidate.exists() {
            continue;
        }
        let zip_path = format!("assets/{}", cleaned);
        if zip.start_file(&zip_path, opts).is_err() {
            continue;
        }
        match read_file_bytes(&candidate.to_string_lossy()) {
            Ok(bytes) => {
                let _ = zip.write_all(&bytes);
                file_count += 1;
            }
            Err(_) => continue,
        }
    }

    // 6. css/js files from the scan
    for f in input.scan.css_files.iter().chain(input.scan.js_files.iter()) {
        let zip_path = format!("assets/{}", f.relative_path);
        if zip.start_file(&zip_path, opts).is_err() {
            continue;
        }
        match read_file(&f.path) {
            Ok(content) => {
                let _ = zip.write_all(content.as_bytes());
                file_count += 1;
            }
            Err(_) => continue,
        }
    }

    let bytes_written = fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);
    zip.finish().map_err(|e| e.to_string())?;
    Ok(ExportResult {
        path: input.output_path,
        bytes_written,
        file_count,
    })
}