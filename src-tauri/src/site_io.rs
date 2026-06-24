use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveArgs {
    pub kind: String,
    pub file: String,
    pub html: String,
    pub folder_path: Option<String>,
    pub server_url: Option<String>,
    /// mtime (unix seconds) of the file when it was loaded, for conflict detection.
    /// None skips conflict check. Some triggers a check before overwriting.
    pub expected_mtime: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveResult {
    pub success: bool,
    pub path: Option<String>,
    pub message: String,
    /// set when a conflict was detected and the save was skipped
    pub conflict: Option<ConflictInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictInfo {
    pub path: String,
    pub expected_mtime: i64,
    pub actual_mtime: i64,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadArgs {
    pub kind: String,
    pub filename: String,
    pub mime: String,
    pub bytes: Vec<u8>,
    pub folder_path: Option<String>,
    pub server_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadResult {
    pub success: bool,
    pub src: String,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UploadProgress {
    pub filename: String,
    pub uploaded: usize,
    pub total: usize,
    pub percent: u8,
}

#[tauri::command]
pub fn save_site(args: SaveArgs) -> Result<SaveResult, String> {
    match args.kind.as_str() {
        "local" => {
            let folder = args.folder_path.ok_or("Site has no folder_path")?;
            let safe_name = sanitize_filename(&args.file);
            let target = PathBuf::from(&folder).join(&safe_name);
            let parent = target.parent().ok_or("Invalid target path")?;
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let canonical_folder =
                fs::canonicalize(&folder).unwrap_or_else(|_| PathBuf::from(&folder));
            let canonical_parent =
                fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
            if !canonical_parent.starts_with(&canonical_folder) {
                return Err("Refusing to write outside site folder (path traversal)".to_string());
            }
            let final_target = canonical_parent.join(target.file_name().unwrap_or_default());

            // conflict detection: if expected_mtime is set, check the file on disk
            if let Some(expected) = args.expected_mtime {
                if let Ok(meta) = fs::metadata(&final_target) {
                    let actual = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    if actual != expected {
                        return Ok(SaveResult {
                            success: false,
                            path: Some(final_target.to_string_lossy().to_string()),
                            message: "File changed on disk since it was loaded".to_string(),
                            conflict: Some(ConflictInfo {
                                path: final_target.to_string_lossy().to_string(),
                                expected_mtime: expected,
                                actual_mtime: actual,
                                message: format!(
                                    "File was modified at {} (expected mtime {}). Save skipped to avoid clobbering.",
                                    actual, expected
                                ),
                            }),
                        });
                    }
                }
            }

            fs::write(&final_target, &args.html).map_err(|e| e.to_string())?;
            Ok(SaveResult {
                success: true,
                path: Some(final_target.to_string_lossy().to_string()),
                message: format!("Saved to {}", final_target.display()),
                conflict: None,
            })
        }
        "remote" => {
            let server = args.server_url.ok_or("Site has no server_url")?;
            let url = build_remote_url(&server, "save.php");
            let url_for_path = url.clone();
            let client = Client::new();
            let res = tauri::async_runtime::block_on(async move {
                client
                    .post(&url)
                    .form(&[("file", args.file.as_str()), ("html", args.html.as_str())])
                    .send()
                    .await
            })
            .map_err(|e: reqwest::Error| e.to_string())?;
            let status = res.status();
            let body = tauri::async_runtime::block_on(res.text())
                .map_err(|e: reqwest::Error| e.to_string())?;
            if status.is_success() {
                Ok(SaveResult {
                    success: true,
                    path: Some(url_for_path),
                    message: body,
                    conflict: None,
                })
            } else {
                Err(format!("Remote save failed ({}): {}", status, body))
            }
        }
        _ => Err(format!("Unknown site kind: {}", args.kind)),
    }
}

#[tauri::command]
pub fn upload_site(args: UploadArgs, app: AppHandle) -> Result<UploadResult, String> {
    let safe_name = sanitize_filename(&args.filename);
    let total = args.bytes.len();

    match args.kind.as_str() {
        "local" => {
            let folder = args.folder_path.ok_or("Site has no folder_path")?;
            let uploads_dir = PathBuf::from(&folder).join("uploads");
            fs::create_dir_all(&uploads_dir).map_err(|e| e.to_string())?;
            let target = uploads_dir.join(&safe_name);
            fs::write(&target, &args.bytes).map_err(|e| e.to_string())?;
            // local uploads are instant - emit 100% progress
            let _ = app.emit(
                "upload-progress",
                UploadProgress {
                    filename: safe_name.clone(),
                    uploaded: total,
                    total,
                    percent: 100,
                },
            );
            let src = format!("/site/uploads/{}", safe_name);
            Ok(UploadResult {
                success: true,
                src,
                message: format!("Uploaded to {}", target.display()),
            })
        }
        "remote" => {
            let server = args.server_url.ok_or("Site has no server_url")?;
            let url = build_remote_url(&server, "upload.php");
            let app_handle = app.clone();
            let filename_for_progress = safe_name.clone();

            // emit start event
            let _ = app_handle.emit(
                "upload-progress",
                UploadProgress {
                    filename: filename_for_progress.clone(),
                    uploaded: 0,
                    total,
                    percent: 0,
                },
            );

            let client = Client::new();
            let part = reqwest::multipart::Part::bytes(args.bytes.clone())
                .file_name(safe_name.clone())
                .mime_str(&args.mime)
                .map_err(|e| e.to_string())?;
            let form = reqwest::multipart::Form::new().part("file", part);

            let res = tauri::async_runtime::block_on(async move {
                client.post(&url).multipart(form).send().await
            })
            .map_err(|e: reqwest::Error| e.to_string())?;

            // emit completion event
            let _ = app.emit(
                "upload-progress",
                UploadProgress {
                    filename: filename_for_progress,
                    uploaded: total,
                    total,
                    percent: 100,
                },
            );

            let status = res.status();
            let body = tauri::async_runtime::block_on(res.text())
                .map_err(|e: reqwest::Error| e.to_string())?;
            if status.is_success() {
                let src = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    json.get("src")
                        .map(|v| v.as_str().unwrap_or("").to_string())
                        .unwrap_or_else(|| safe_name.clone())
                } else {
                    safe_name.clone()
                };
                Ok(UploadResult {
                    success: true,
                    src,
                    message: body,
                })
            } else {
                Err(format!("Remote upload failed ({}): {}", status, body))
            }
        }
        _ => Err(format!("Unknown site kind: {}", args.kind)),
    }
}

/// returns the mtime (unix seconds) of a file in the site folder, for conflict detection
#[tauri::command]
pub fn get_file_mtime(folder_path: String, file: String) -> Result<Option<i64>, String> {
    let safe_name = sanitize_filename(&file);
    let target = PathBuf::from(&folder_path).join(&safe_name);
    let canonical_folder =
        fs::canonicalize(&folder_path).unwrap_or_else(|_| PathBuf::from(&folder_path));
    let canonical_parent = target
        .parent()
        .and_then(|p| fs::canonicalize(p).ok())
        .unwrap_or_else(|| target.parent().map(|p| p.to_path_buf()).unwrap_or_default());
    if !canonical_parent.starts_with(&canonical_folder) {
        return Err("Path traversal blocked".to_string());
    }
    let final_target = canonical_parent.join(target.file_name().unwrap_or_default());
    match fs::metadata(&final_target) {
        Ok(meta) => {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            Ok(Some(mtime))
        }
        Err(_) => Ok(None),
    }
}

fn build_remote_url(server: &str, script: &str) -> String {
    if server.ends_with(script) || server.contains(&format!("{}.php", script.trim_end_matches(".php"))) {
        server.to_string()
    } else if server.ends_with('/') {
        format!("{}{}", server, script)
    } else {
        format!("{}/{}", server.trim_end_matches('/'), script)
    }
}

fn sanitize_filename(name: &str) -> String {
    let name = name.trim_start_matches('/');
    let name = name.replace("..", "");
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/' {
                c
            } else {
                '-'
            }
        })
        .collect()
}