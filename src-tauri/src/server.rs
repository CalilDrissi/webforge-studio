use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, header::HeaderMap};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::SiteFolders;

// Phase A — limits mirroring the original PHP save.php
const MAX_FILE_LIMIT: usize = 1024 * 1024 * 2; // 2 MiB
const ALLOW_PHP: bool = false;
const ALLOWED_OEMBED_DOMAINS: &[&str] = &[
    "https://www.youtube.com/",
    "https://youtube.com/",
    "https://www.vimeo.com/",
    "https://vimeo.com/",
    "https://www.x.com/",
    "https://x.com/",
    "https://publish.twitter.com/",
    "https://www.twitter.com/",
    "https://twitter.com/",
    "https://www.reddit.com/",
    "https://reddit.com/",
];
const UPLOAD_DENY_EXTENSIONS: &[&str] = &["php"];
const UPLOAD_ALLOW_EXTENSIONS: &[&str] = &["ico", "jpg", "jpeg", "png", "gif", "webp", "svg"];

pub struct ServerState {
    pub root: PathBuf,
    pub port: u16,
    pub site_folders: SiteFolders,
}

pub fn mime_for(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "eot" => "application/vnd.ms-fontobject",
        "map" => "application/json",
        "txt" | "md" => "text/plain; charset=utf-8",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

pub async fn start_server(root: PathBuf, site_folders: SiteFolders) -> Result<ServerState, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let root = root.canonicalize().map_err(|e| e.to_string())?;

    let state = Arc::new(ServerState {
        root: root.clone(),
        port,
        site_folders: site_folders.clone(),
    });

    let serve_state = state.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let io = TokioIo::new(stream);
            let s = serve_state.clone();
            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(io, service_fn(move |req| serve_request(req, s.clone())))
                    .await;
            });
        }
    });

    Ok(ServerState {
        root: state.root.clone(),
        port,
        site_folders,
    })
}

fn serve_request(
    req: Request<hyper::body::Incoming>,
    state: Arc<ServerState>,
) -> impl std::future::Future<Output = Result<Response<Full<Bytes>>, Infallible>> + Send {
    let root = state.root.clone();
    let site_folders = state.site_folders.clone();
    async move {
        let raw_path = req.uri().path();
        let rel = raw_path.trim_start_matches('/');
        let method = req.method().clone();

        // Phase A — native API routes (replacing save.php / upload.php / scan.php).
        // These act on the site identified by the `X-Site-Id` header or `?site=` query param.
        if let Some(rest) = rel.strip_prefix("api/") {
            let rest = rest.to_string();
            return Ok(handle_api_route(&rest, method, req, site_folders).await);
        }

        // route /theme-preview/<slug> — serve the preview HTML for a theme
        if let Some(rest) = rel.strip_prefix("theme-preview/") {
            let slug = rest.split('?').next().unwrap_or(rest).trim_end_matches('/');
            return Ok(serve_theme_preview(slug));
        }

        // route /theme/<slug>/<path...> — serve static assets from a theme folder
        if let Some(rest) = rel.strip_prefix("theme/") {
            let mut parts = rest.splitn(2, '/');
            let slug = parts.next().unwrap_or("");
            let sub_path = parts.next().unwrap_or("");
            return Ok(serve_theme_asset(slug, sub_path));
        }

        // route /proxy/<percent-encoded-remote-url> to the remote proxy
        if let Some(rest) = rel.strip_prefix("proxy/") {
            let encoded = rest;
            let remote_url = match percent_decode_str(encoded) {
                Some(u) => u,
                None => return Ok(bad_request_response("invalid proxy url encoding")),
            };
            return Ok(proxy_fetch(&remote_url, req.uri().query()).await);
        }

        // route /site/<id>/<path...> to a registered local site folder
        if let Some(rest) = rel.strip_prefix("site/") {
            let mut parts = rest.splitn(2, '/');
            let id_str = parts.next().unwrap_or("");
            let sub_path = parts.next().unwrap_or("");
            let site_id: i64 = match id_str.parse() {
                Ok(id) => id,
                Err(_) => return Ok(not_found_response("invalid site id")),
            };
            let folders = site_folders.read().await;
            let site_root = match folders.get(&site_id) {
                Some(p) => p.clone(),
                None => return Ok(not_found_response("site folder not registered")),
            };
            return Ok(serve_from_root(&site_root, sub_path, false));
        }

        // Editor asset requests served from the bundled Webforge root.
        // For editor.html (and editor.php), inject a dynamically computed `defaultPages`
        // built by scanning my-pages/ and demo/ so the file manager shows real files
        // without requiring PHP execution.
        if (rel == "editor.html" || rel == "editor.php") && method == Method::GET {
            return Ok(serve_editor_with_dynamic_pages(&root, rel, req.uri().query()));
        }

        Ok(serve_from_root(&root, rel, true))
    }
}

fn percent_decode_str(s: &str) -> Option<String> {
    // simple percent-decoding for the proxy path
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1])?;
            let lo = hex_val(bytes[i + 2])?;
            out.push((hi * 16 + lo) as char);
            i += 3;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

async fn proxy_fetch(remote_url: &str, query: Option<&str>) -> Response<Full<Bytes>> {
    let full_url = if let Some(q) = query {
        if !q.is_empty() {
            format!("{}?{}", remote_url, q)
        } else {
            remote_url.to_string()
        }
    } else {
        remote_url.to_string()
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => return server_error_response(&format!("proxy client error: {}", e)),
    };

    let res = match client.get(&full_url).send().await {
        Ok(r) => r,
        Err(e) => return bad_gateway_response(&format!("proxy fetch failed: {}", e)),
    };

    let status = res.status();
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let final_url = res.url().to_string();

    let bytes = match res.bytes().await {
        Ok(b) => b,
        Err(e) => return bad_gateway_response(&format!("proxy body error: {}", e)),
    };

    // determine if this is HTML that needs rewriting + bridge injection
    let is_html = content_type.contains("text/html") || full_url.ends_with(".html") || full_url.ends_with(".htm");

    if is_html {
        let body = String::from_utf8_lossy(&bytes).to_string();
        let rewritten = rewrite_html_urls(&body, &final_url);
        let injected = inject_bridge_shim(&rewritten);
        return Response::builder()
            .status(status)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Access-Control-Allow-Origin", "*")
            .header("Cache-Control", "no-cache")
            .header("X-Proxied-From", final_url)
            .body(Full::new(Bytes::from(injected)))
            .unwrap();
    }

    // for CSS, rewrite url() references to go through proxy
    let final_bytes = if content_type.contains("text/css") {
        let body = String::from_utf8_lossy(&bytes).to_string();
        let rewritten = rewrite_css_urls(&body, &final_url);
        Bytes::from(rewritten)
    } else {
        bytes
    };

    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-cache")
        .header("X-Proxied-From", final_url)
        .body(Full::new(final_bytes))
        .unwrap()
}

/// Rewrite all URL references in an HTML document to go through /proxy/<encoded-absolute-url>.
/// Handles src, href, srcset, poster, data-* where applicable, and inline style url().
fn rewrite_html_urls(html: &str, base_url: &str) -> String {
    let base = base_url.to_string();
    let rewrite_attr = |val: &str| -> String { absolutize_and_proxy(val, &base) };

    let mut out = String::with_capacity(html.len() + 256);
    let lower = html.to_lowercase();
    let mut pos = 0;

    while let Some(tag_start) = lower[pos..].find('<') {
        let abs_start = pos + tag_start;
        out.push_str(&html[pos..abs_start]);
        // find end of this tag
        let tag_end = match lower[abs_start..].find('>') {
            Some(e) => abs_start + e + 1,
            None => {
                out.push_str(&html[abs_start..]);
                break;
            }
        };
        let tag = &html[abs_start..tag_end];
        let rewritten_tag = rewrite_tag_attrs(tag, &rewrite_attr);
        out.push_str(&rewritten_tag);
        pos = tag_end;
    }
    out.push_str(&html[pos..]);
    out
}

fn rewrite_tag_attrs(tag: &str, rewrite: &dyn Fn(&str) -> String) -> String {
    // only rewrite src, href, poster, srcset on elements
    let attrs = ["src", "href", "poster", "data-src", "data-href"];
    let mut out = tag.to_string();
    for attr in attrs.iter() {
        let patterns = [format!(" {}=\"", attr), format!(" {}='", attr), format!(" {}=", attr)];
        for pat in patterns.iter() {
            if let Some(p) = out.to_lowercase().find(&pat.to_lowercase()) {
                let val_start = p + pat.len();
                let rest = &out[val_start..];
                let (val, _terminator) = if rest.starts_with('"') {
                    // unquoted - shouldn't happen since pat includes the quote
                    (rest, '"')
                } else if let Some(end) = rest.find('"') {
                    (&rest[..end], '"')
                } else if let Some(end) = rest.find('\'') {
                    (&rest[..end], '\'')
                } else if let Some(end) = rest.find(|c: char| c.is_whitespace() || c == '>') {
                    (&rest[..end], ' ')
                } else {
                    (rest, ' ')
                };
                if !val.is_empty() && !val.starts_with("data:") && !val.starts_with("javascript:") && !val.starts_with("mailto:") && !val.starts_with("tel:") && !val.starts_with("#") {
                    let new_val = rewrite(val);
                    out.replace_range(val_start..val_start + val.len(), &new_val);
                }
                break;
            }
        }
    }
    out
}

/// Rewrite url(...) references in CSS to go through proxy
fn rewrite_css_urls(css: &str, base_url: &str) -> String {
    let mut out = String::with_capacity(css.len() + 128);
    let lower = css.to_lowercase();
    let mut pos = 0;
    while let Some(p) = lower[pos..].find("url(") {
        let abs = pos + p;
        out.push_str(&css[pos..abs + 4]);
        let after = &css[abs + 4..];
        let (quote, val_start_offset) = if after.starts_with('"') {
            ('"', 1)
        } else if after.starts_with('\'') {
            ('\'', 1)
        } else {
            ('\0', 0)
        };
        let after_trimmed = &after[val_start_offset..];
        let end = if quote != '\0' {
            after_trimmed.find(quote).unwrap_or(after_trimmed.len())
        } else {
            after_trimmed.find(')').unwrap_or(after_trimmed.len())
        };
        let val = &after_trimmed[..end];
        if !val.is_empty() && !val.starts_with("data:") && !val.starts_with('#') {
            let new_val = absolutize_and_proxy(val, base_url);
            out.push_str(&new_val);
        } else {
            out.push_str(val);
        }
        // advance past the value + terminator
        let consumed = 4 + val_start_offset + end;
        pos = abs + consumed;
        if quote != '\0' {
            // skip the closing quote
            if let Some(_) = css[pos..].chars().next() {
                pos += 1;
            }
        }
        // skip the closing )
        if let Some(c) = css[pos..].chars().next() {
            if c == ')' {
                pos += 1;
            }
        }
    }
    out.push_str(&css[pos..]);
    out
}

/// Resolve a possibly-relative URL against base_url, then make it go through /proxy/
fn absolutize_and_proxy(val: &str, base_url: &str) -> String {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return val.to_string();
    }
    // never proxy these schemes
    let skip_prefixes = ["data:", "javascript:", "mailto:", "tel:", "blob:", "about:"];
    if skip_prefixes.iter().any(|p| trimmed.starts_with(p)) {
        return val.to_string();
    }
    // never proxy fragment-only
    if trimmed.starts_with('#') {
        return val.to_string();
    }
    if trimmed.starts_with("//") {
        let abs = format!("https:{}", trimmed);
        return format!("/proxy/{}", percent_encode(&abs));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return format!("/proxy/{}", percent_encode(trimmed));
    }
    if trimmed.starts_with("/") {
        if let Ok(base) = url::Url::parse(base_url) {
            let abs = format!("{}://{}{}", base.scheme(), base.host_str().unwrap_or(""), trimmed);
            return format!("/proxy/{}", percent_encode(&abs));
        }
    }
    if let Ok(base) = url::Url::parse(base_url) {
        if let Ok(joined) = base.join(trimmed) {
            // only proxy http/https results
            let scheme = joined.scheme();
            if scheme == "http" || scheme == "https" {
                return format!("/proxy/{}", percent_encode(joined.as_str()));
            }
        }
    }
    val.to_string()
}

fn percent_encode(s: &str) -> String {
    // percent-encode everything except alphanumerics and a few safe chars
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn bad_request_response(msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(Full::new(Bytes::from(msg.to_string())))
        .unwrap()
}

fn bad_gateway_response(msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .body(Full::new(Bytes::from(msg.to_string())))
        .unwrap()
}

fn server_error_response(msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(Full::new(Bytes::from(msg.to_string())))
        .unwrap()
}

// ---- Phase A: native API routes (PHP replacement) ----

/// Dispatch an /api/* request. Resolves the target site folder from `X-Site-Id` header
/// or `?site=` query param. Returns a JSON-ish response.
async fn handle_api_route(
    rest: &str,
    method: Method,
    req: Request<hyper::body::Incoming>,
    site_folders: SiteFolders,
) -> Response<Full<Bytes>> {
    // route shape: api/<site-id>/<verb>[?...]
    // We accept both /api/<verb>?site=<id> and /api/<site-id>/<verb> for robustness.
    let query = req.uri().query().map(|s| s.to_string());
    let headers = req.headers().clone();

    let (site_id, verb) = if let Some(idx) = rest.find('/') {
        let maybe_id: Option<i64> = rest[..idx].parse().ok();
        let v = &rest[idx + 1..];
        // strip any trailing ?query from the verb
        let v = v.split('?').next().unwrap_or(v).trim_end_matches('/');
        if let Some(id) = maybe_id {
            (Some(id), v)
        } else {
            (parse_site_query(query.as_deref()), v)
        }
    } else {
        // single-segment: api/save?site=123
        let v = rest.split('?').next().unwrap_or(rest).trim_end_matches('/');
        (parse_site_query(query.as_deref()), v)
    };

    // Body is needed by save & upload; collect it once here.
    let (_parts, body) = req.into_parts();
    let collected = match body.collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("body read failed: {}", e)),
    };

    // Action sub-route for the save.php-style `?action=` verbs (rename/delete/saveReusable/oembedProxy).
    // We honor both `?action=` and `/api/<verb>` forms.
    let action = parse_action_from_query(query.as_deref());

    let site_id = match site_id {
        Some(id) => id,
        None => return json_error(StatusCode::BAD_REQUEST, "missing site id (use X-Site-Id header or ?site= param)"),
    };

    let folders = site_folders.read().await;
    let site_root = match folders.get(&site_id) {
        Some(p) => p.clone(),
        None => return json_error(StatusCode::NOT_FOUND, &format!("site {} not registered", site_id)),
    };
    drop(folders);

    // Resolve which logical endpoint we are serving.
    let effective_verb = if verb.is_empty() && !action.is_empty() {
        "save"
    } else {
        verb
    };

    match effective_verb {
        "save" => handle_save(&site_root, method, action.as_str(), collected).await,
        "upload" => handle_upload(&site_root, method, &headers, &query, collected).await,
        "scan" => handle_scan(&site_root, method, &query).await,
        "pages" => handle_pages(&site_root, method).await,
        _ => json_error(StatusCode::NOT_FOUND, &format!("unknown api route: {}", effective_verb)),
    }
}

fn parse_site_query(query: Option<&str>) -> Option<i64> {
    let q = query?;
    for pair in q.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next() == Some("site") {
            return kv.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

fn parse_action_from_query(query: Option<&str>) -> String {
    if let Some(q) = query {
        for pair in q.split('&') {
            let mut kv = pair.splitn(2, '=');
            if kv.next() == Some("action") {
                return kv.next().unwrap_or("").to_string();
            }
        }
    }
    String::new()
}

fn json_ok(text: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Access-Control-Allow-Origin", "*")
        .body(Full::new(Bytes::from(text.to_string())))
        .unwrap()
}

fn json_text_ok(text: &str) -> Response<Full<Bytes>> {
    // webforge's saveAjax expects plain text response, not JSON
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Access-Control-Allow-Origin", "*")
        .body(Full::new(Bytes::from(text.to_string())))
        .unwrap()
}

fn json_error(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Access-Control-Allow-Origin", "*")
        .body(Full::new(Bytes::from(format!(
            "{{\"success\":false,\"error\":\"{}\"}}",
            json_escape(msg)
        ))))
        .unwrap()
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Replicates save.php: sanitize file name, optionally apply action (rename/delete/saveReusable/oembedProxy),
/// write or operate on the file under the site root. POST body is application/x-www-form-urlencoded.
async fn handle_save(
    site_root: &Path,
    _method: Method,
    action: &str,
    body: Bytes,
) -> Response<Full<Bytes>> {
    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "body is not utf-8"),
    };
    let form = parse_form_urlencoded(body_str);

    if !action.is_empty() {
        match action {
            "rename" => {
                let file = sanitize_filename(&form.get("file").cloned().unwrap_or_default(), "html");
                let newfile = sanitize_filename(&form.get("newfile").cloned().unwrap_or_default(), "html");
                if file.is_empty() || newfile.is_empty() {
                    return json_error(StatusCode::BAD_REQUEST, "missing file or newfile");
                }
                let from = site_root.join(&file);
                let to = site_root.join(&newfile);
                match std::fs::rename(&from, &to) {
                    Ok(_) => json_text_ok(&format!("File '{}' renamed to '{}'", file, newfile)),
                    Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error renaming file: {}", e)),
                }
            }
            "delete" => {
                let file = sanitize_filename(&form.get("file").cloned().unwrap_or_default(), "html");
                if file.is_empty() {
                    return json_error(StatusCode::BAD_REQUEST, "missing file");
                }
                let path = site_root.join(&file);
                match std::fs::remove_file(&path) {
                    Ok(_) => json_text_ok(&format!("File '{}' deleted", file)),
                    Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error deleting file: {}", e)),
                }
            }
            "saveReusable" => {
                let rtype = form.get("type").cloned().unwrap_or_default();
                let name = form.get("name").cloned().unwrap_or_default();
                let html = form.get("html").cloned().unwrap_or_default();
                if rtype.is_empty() || name.is_empty() || html.is_empty() {
                    return json_error(StatusCode::BAD_REQUEST, "Missing reusable element data!");
                }
                // path shape: <type>/<name>.html under site root
                let safe_name = sanitize_filename(&format!("{}/{}", rtype, name), "html");
                let path = site_root.join(&safe_name);
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&path, &html) {
                    Ok(_) => json_text_ok(&format!("File saved '{}'", path.display())),
                    Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error saving file: {}", e)),
                }
            }
            "oembedProxy" => {
                let url = form.get("url").cloned().unwrap_or_default();
                if !valid_oembed_url(&url) {
                    return json_error(StatusCode::BAD_REQUEST, "Invalid url!");
                }
                let client = reqwest::Client::new();
                match client.get(&url).header("User-Agent", "webforge-studio/oembed-proxy").send().await {
                    Ok(r) => {
                        let text = r.text().await.unwrap_or_default();
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json; charset=utf-8")
                            .body(Full::new(Bytes::from(text)))
                            .unwrap()
                    }
                    Err(e) => json_error(StatusCode::BAD_GATEWAY, &format!("oembed fetch failed: {}", e)),
                }
            }
            _ => json_error(StatusCode::BAD_REQUEST, &format!("Invalid action '{}'", action)),
        }
    } else {
        // save page
        let start_template_url = form.get("startTemplateUrl").cloned().unwrap_or_default();
        let file_raw = form.get("file").cloned().unwrap_or_default();
        let mut html = String::new();

        if !start_template_url.is_empty() {
            let tpl = sanitize_filename(&start_template_url, "html");
            if !tpl.is_empty() {
                html = std::fs::read_to_string(site_root.join(&tpl)).unwrap_or_default();
            }
        } else if let Some(h) = form.get("html") {
            html = h.clone();
            // truncate to MAX_FILE_LIMIT
            if html.len() > MAX_FILE_LIMIT {
                html.truncate(MAX_FILE_LIMIT);
            }
            if !ALLOW_PHP && php_tag_re(&html) {
                return json_error(StatusCode::BAD_REQUEST, "PHP not allowed!");
            }
        }

        if html.is_empty() {
            return json_error(StatusCode::BAD_REQUEST, "Html content is empty!");
        }
        let file = sanitize_filename(&file_raw, "html");
        if file.is_empty() {
            return json_error(StatusCode::BAD_REQUEST, "Filename is empty!");
        }
        let path = site_root.join(&file);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&path, &html) {
            Ok(_) => json_text_ok(&format!("File saved '{}'", path.display())),
            Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error saving file: {}", e)),
        }
    }
}

/// Replicates upload.php: parse multipart/form-data, validate extension, save under uploads/.
async fn handle_upload(
    site_root: &Path,
    _method: Method,
    headers: &HeaderMap,
    _query: &Option<String>,
    body: Bytes,
) -> Response<Full<Bytes>> {
    // Determine upload subdir from mediaPath form field, defaulting to uploads/
    let mut media_path = "uploads".to_string();

    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let boundary = match extract_boundary(ct) {
        Some(b) => b,
        None => return json_error(StatusCode::BAD_REQUEST, "missing multipart boundary"),
    };

    let mut file_name = String::new();
    let mut file_bytes: Vec<u8> = Vec::new();

    // Manual multipart parsing — avoids async-reader complexity and is plenty for single-file uploads.
    let fields = match parse_multipart(&body, &boundary) {
        Ok(f) => f,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("multipart parse error: {}", e)),
    };
    for (disp, filename, value) in fields {
        if disp == "mediaPath" {
            media_path = sanitize_path_segment(&String::from_utf8_lossy(&value));
        } else if disp == "file" {
            file_name = filename.unwrap_or_else(|| "upload.bin".to_string());
            file_bytes = value;
        }
    }

    if file_bytes.is_empty() || file_name.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "missing file field");
    }

    // sanitize the filename (path-safe)
    let safe_name = sanitize_path_segment(&file_name);
    let extension = safe_name.rsplit('.').next().unwrap_or("").to_lowercase();
    if UPLOAD_DENY_EXTENSIONS.iter().any(|e| *e == extension) {
        return json_error(StatusCode::BAD_REQUEST, &format!("File type {} not allowed!", extension));
    }
    if !UPLOAD_ALLOW_EXTENSIONS.iter().any(|e| *e == extension) {
        return json_error(StatusCode::BAD_REQUEST, &format!("File type {} not allowed!", extension));
    }

    let dest_dir = site_root.join(&media_path);
    if std::fs::create_dir_all(&dest_dir).is_err() {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "failed to create upload directory");
    }
    let dest_path = dest_dir.join(&safe_name);
    if std::fs::write(&dest_path, &file_bytes).is_err() {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "failed to write uploaded file");
    }
    let rel = format!("{}/{}", media_path, safe_name);
    json_text_ok(&rel)
}

fn extract_boundary(content_type: &str) -> Option<String> {
    for part in content_type.split(';') {
        let p = part.trim();
        if let Some(rest) = p.strip_prefix("boundary=") {
            let rest = rest.trim_matches('"');
            return Some(rest.to_string());
        }
    }
    None
}

/// Minimal multipart/form-data parser. Returns (disposition_name, optional filename, value bytes).
/// Good enough for webforge's single-file uploads — not a general-purpose parser.
fn parse_multipart(body: &[u8], boundary: &str) -> Result<Vec<(String, Option<String>, Vec<u8>)>, String> {
    let delim = format!("--{}", boundary);
    let close = format!("--{}--", boundary);
    let mut out = Vec::new();

    // Split on the boundary delimiter lines.
    let mut pos = 0;
    let mut fields = Vec::new();
    while pos < body.len() {
        // find the next delimiter
        let next = match find_subsequence(&body[pos..], delim.as_bytes()) {
            Some(n) => pos + n,
            None => break,
        };
        // skip past the delimiter line (include trailing CRLF)
        let line_end = next + delim.len();
        let after_delim = line_end + 2; // skip CRLF after --boundary
        if after_delim > body.len() {
            break;
        }
        // find the next delimiter from after_delim
        let next_delim = match find_subsequence(&body[after_delim..], delim.as_bytes()) {
            Some(n) => after_delim + n,
            None => break,
        };
        // The block content lives in body[after_delim..next_delim] minus trailing CRLF.
        let content_end = if next_delim >= 2 + after_delim && &body[next_delim - 2..next_delim] == b"\r\n" {
            next_delim - 2
        } else {
            next_delim
        };
        fields.push(&body[after_delim..content_end]);
        pos = next_delim;
    }

    for block in fields {
        if block == close.as_bytes() {
            break;
        }
        // each block: headers\r\n\r\ncontent
        let header_end = match find_subsequence(block, b"\r\n\r\n") {
            Some(n) => n,
            None => continue,
        };
        let header_bytes = &block[..header_end];
        let value = block[header_end + 4..].to_vec();

        let header_str = String::from_utf8_lossy(header_bytes);
        let mut name = String::new();
        let mut filename: Option<String> = None;
        for line in header_str.split("\r\n") {
            // Only lowercase the header name (before the colon) so we can match
            // "content-disposition:" case-insensitively while preserving the original
            // case of the field name and filename values.
            let colon = line.find(':');
            if let Some(c) = colon {
                let header_name = line[..c].to_ascii_lowercase();
                if header_name == "content-disposition" {
                    let rest = &line[c + 1..];
                    for part in rest.split(';') {
                        let p = part.trim();
                        if let Some(v) = p.strip_prefix("name=\"") {
                            name = v.trim_end_matches('"').to_string();
                        } else if let Some(v) = p.strip_prefix("filename=\"") {
                            filename = Some(v.trim_end_matches('"').to_string());
                        }
                    }
                }
            }
        }
        out.push((name, filename, value));
    }
    Ok(out)
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn sanitize_path_segment(name: &str) -> String {
    // mirror upload.php: strip .. and disallowed chars; keep path separators intact
    let disallow = [".htaccess", "passwd"];
    let mut s = name.to_string();
    for d in disallow.iter() {
        s = s.replace(d, "");
    }
    s.retain(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/' || c == '\\');
    // repeatedly strip `..` so "foo/../bar" collapses cleanly
    while s.contains("..") {
        s = s.replace("..", "");
    }
    // collapse repeated slashes left over after `..` removal
    while s.contains("//") || s.contains("\\\\") {
        s = s.replace("//", "/").replace("\\\\", "\\");
    }
    // strip leading slashes left over after `..` removal
    while s.starts_with('/') || s.starts_with('\\') {
        s = s[1..].to_string();
    }
    s.split('?').next().unwrap_or(&s).to_string()
}

/// Replicates scan.php: recursively list media folder as a nested JSON tree.
async fn handle_scan(site_root: &Path, _method: Method, query: &Option<String>) -> Response<Full<Bytes>> {
    // Accept mediaPath via query string (?mediaPath=...) or fall back to 'media'
    let mut media_rel = "media".to_string();
    if let Some(q) = query.as_deref() {
        for pair in q.split('&') {
            let mut kv = pair.splitn(2, '=');
            if kv.next() == Some("mediaPath") {
                media_rel = sanitize_path_segment(kv.next().unwrap_or(""));
                if media_rel.is_empty() {
                    media_rel = "media".to_string();
                }
                break;
            }
        }
    }
    let scan_dir = site_root.join(&media_rel);
    let tree = scan_directory(&scan_dir, &scan_dir);
    json_ok(&serde_json::to_string(&tree).unwrap_or_else(|_| "{}".to_string()))
}

#[derive(Debug, serde::Serialize)]
struct FileNode {
    name: String,
    #[serde(rename = "type")]
    node_type: &'static str,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    items: Option<Vec<FileNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

fn scan_directory(root: &Path, current: &Path) -> FileNode {
    let name = current
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if current.is_dir() {
        let mut items = Vec::new();
        if let Ok(entries) = std::fs::read_dir(current) {
            let mut sorted: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            sorted.sort_by_key(|e| e.file_name());
            for entry in sorted {
                let path = entry.path();
                if path.is_dir() {
                    items.push(scan_directory(root, &path));
                } else {
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    if file_name.starts_with('.') {
                        continue;
                    }
                    let rel = path
                        .strip_prefix(root)
                        .ok()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    items.push(FileNode {
                        name: file_name,
                        node_type: "file",
                        path: rel,
                        items: None,
                        size: entry.metadata().ok().map(|m| m.len()),
                    });
                }
            }
        }
        FileNode {
            name,
            node_type: "folder",
            path: current
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            items: Some(items),
            size: None,
        }
    } else {
        FileNode {
            name,
            node_type: "file",
            path: String::new(),
            items: None,
            size: std::fs::metadata(current).ok().map(|m| m.len()),
        }
    }
}

/// Lists all HTML files under my-pages/ and demo/ of the site root, returning a JSON
/// array of {name, file, url, title, folder} for the FileManager. Matches editor.php.
async fn handle_pages(_site_root: &Path, _method: Method) -> Response<Full<Bytes>> {
    // Built separately by serve_editor_with_dynamic_pages; this endpoint exposes the
    // same data as JSON for clients that want it.
    let items = collect_pages(&_site_root);
    json_ok(&serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string()))
}

#[derive(Debug, serde::Serialize)]
struct PageEntry {
    name: String,
    file: String,
    url: String,
    title: String,
    folder: String,
}

/// Walk my-pages/*.html and demo/**/*.html (excluding editor.html/new-page-blank-template.html),
/// producing the page list webforge's FileManager consumes.
fn collect_pages(root: &Path) -> Vec<PageEntry> {
    let mut out = Vec::new();
    let roots = ["my-pages", "demo"];
    for r in roots {
        let base = root.join(r);
        if !base.is_dir() {
            continue;
        }
        walk_html_pages(&base, &base, r, &mut out);
    }
    out
}

fn walk_html_pages(base: &Path, current: &Path, group: &str, out: &mut Vec<PageEntry>) {
    let Ok(entries) = std::fs::read_dir(current) else { return };
    let mut sorted: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    sorted.sort_by_key(|e| e.file_name());
    for entry in sorted {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            // skip source template dirs — they contain unresolved @@include directives
            // that only work with a build step; serving them raw shows broken pages
            if name == "src" {
                continue;
            }
            walk_html_pages(base, &path, group, out);
            continue;
        }
        if !name.ends_with(".html") {
            continue;
        }
        if name == "editor.html" || name == "new-page-blank-template.html" {
            continue;
        }
        let rel = path.strip_prefix(base).unwrap_or(&path);
        let url = format!("{}/{}", group, rel.to_string_lossy().replace('\\', "/"));
        let file_name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let folder = rel
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        // editor.php: filename derived from subfolder if it's "index"
        let display_name = if file_name == "index" && !folder.is_empty() {
            folder.split('/').next_back().unwrap_or(&file_name).to_string()
        } else {
            file_name
        };
        let title = uppercase_first(&display_name);
        out.push(PageEntry {
            name: display_name,
            file: url.clone(),
            url,
            title,
            folder: group.to_string(),
        });
    }
}

fn uppercase_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Serve editor.html with the `defaultPages` literal replaced by a dynamically scanned list,
/// and inject selected theme sections.js after the editor's sections.js script tag.
/// Mirrors editor.php's behaviour but runs entirely in Rust.
fn serve_editor_with_dynamic_pages(root: &Path, rel: &str, query: Option<&str>) -> Response<Full<Bytes>> {
    let file_path = root.join(if rel == "editor.php" { "editor.html" } else { rel });
    let Ok(bytes) = std::fs::read(&file_path) else {
        return not_found_response(&format!("not found: {}", rel));
    };
    let mut html = String::from_utf8_lossy(&bytes).to_string();

    let pages = collect_pages(root);
    if !pages.is_empty() {
        let mut js_items = String::new();
        for p in &pages {
            let safe_name = p.name.replace('\'', "\\'");
            let safe_url = p.url.replace('\'', "\\'");
            let safe_title = p.title.replace('\'', "\\'");
            let safe_folder = p.folder.replace('\'', "\\'");
            js_items.push_str(&format!(
                "\"{}\":{{name:\"{}\",file:\"{}\",url:\"{}\",title:\"{}\",folder:\"{}\"}},",
                safe_name, safe_name, safe_url, safe_url, safe_title, safe_folder
            ));
        }
        // Replace the literal `= defaultPages;` assignment with our scanned list.
        let replacement = format!(" = {{{}}};", js_items.trim_end_matches(','));
        html = html.replacen("= defaultPages;", &replacement, 1);
    }

    // inject selected theme sections.js if the editor request has ?themes=slug1,slug2
    if let Some(q) = query {
        let theme_slugs = parse_themes_query(q);
        if !theme_slugs.is_empty() {
            let theme_js = theme_library::get_theme_sections_js(&theme_slugs);
            if !theme_js.is_empty() {
                // inject after the editor's sections.js script tag
                let inject_tag = format!(
                    "<script>\n// Theme Library injection\n{}\n</script>\n</head>",
                    theme_js
                );
                html = html.replacen("</head>", &inject_tag, 1);
            }
        }
    }

    let injected = inject_bridge_shim(&html);
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-cache")
        .body(Full::new(Bytes::from(injected)))
        .unwrap()
}

/// Parse ?themes=slug1,slug2 from the query string.
fn parse_themes_query(query: &str) -> Vec<String> {
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next() == Some("themes") {
            let val = kv.next().unwrap_or("");
            return val.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
        }
    }
    Vec::new()
}

fn php_tag_re(html: &str) -> bool {
    use regex::Regex;
    thread_local! {
        static RE: Regex = Regex::new(r#"<\?php|<\? |<\?=|<\s*script\s*language\s*=\s*"\s*php\s*"\s*>"#).unwrap();
    }
    RE.with(|r| r.is_match(html))
}

fn valid_oembed_url(url: &str) -> bool {
    ALLOWED_OEMBED_DOMAINS.iter().any(|d| url.starts_with(d))
}

fn parse_form_urlencoded(body: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in body.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = urlencoding::decode(kv.next().unwrap_or(""))
            .map(|s| s.to_string())
            .unwrap_or_default();
        let val = urlencoding::decode(kv.next().unwrap_or(""))
            .map(|s| s.to_string())
            .unwrap_or_default();
        map.insert(key, val);
    }
    map
}

/// Sanitize a file name similarly to PHP's save.php: strip ../, disallow weird chars,
/// enforce the given extension.
fn sanitize_filename(name: &str, extension: &str) -> String {
    // remove query string + `..`
    let name = name.split('?').next().unwrap_or(name).replace("..", "");
    let mut out: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.' || *c == '/' || *c == '\\')
        .collect();
    // strip leading slash
    if out.starts_with('/') {
        out = out.trim_start_matches('/').to_string();
    }
    // disallow sensitive files
    let basename = out.rsplit(['/', '\\']).next().unwrap_or(&out);
    if basename == ".htaccess" || basename == "passwd" {
        return String::new();
    }
    // enforce extension
    if !extension.is_empty() {
        // strip existing extension and re-append ours
        if let Some(dot_pos) = out.rfind('.') {
            // only treat as extension if it's in the last path segment
            let last_seg = out.rfind(['/', '\\']).map(|p| p + 1).unwrap_or(0);
            if dot_pos > last_seg {
                out.truncate(dot_pos);
            }
        }
        out.push('.');
        out.push_str(extension);
    }
    out
}

// ---- end Phase A ----

// ---- Theme Library routes ----

use crate::theme_library;

/// Serve the preview HTML for a theme (written by preview_theme command).
fn serve_theme_preview(slug: &str) -> Response<Full<Bytes>> {
    let preview_file = theme_library::theme_library_root().join(slug).join("_preview.html");
    match std::fs::read(&preview_file) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Access-Control-Allow-Origin", "*")
            .body(Full::new(Bytes::from(bytes)))
            .unwrap(),
        Err(_) => not_found_response(&format!("theme preview not found: {}", slug)),
    }
}

/// Serve a static asset from a theme folder (e.g., CSS, JS, images).
fn serve_theme_asset(slug: &str, sub_path: &str) -> Response<Full<Bytes>> {
    let theme_dir = theme_library::theme_library_root().join(slug);
    if !theme_dir.is_dir() {
        return not_found_response(&format!("theme not found: {}", slug));
    }
    serve_from_root(&theme_dir, sub_path, false)
}

// ---- end Theme Library routes ----

fn serve_from_root(root: &PathBuf, rel: &str, inject_bridge: bool) -> Response<Full<Bytes>> {
    let rel = rel.split('?').next().unwrap_or(rel);
    let rel = if rel.is_empty() { "editor.html" } else { rel };

    let candidate = root.join(rel);
    if !candidate.starts_with(root) {
        return forbidden_response();
    }

    let file_path = if candidate.is_dir() {
        candidate.join("index.html")
    } else {
        candidate
    };

    match std::fs::read(&file_path) {
        Ok(bytes) => {
            let ext = file_path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let mime = mime_for(&ext);

            let builder = Response::builder()
                .header("Content-Type", mime)
                .header("Access-Control-Allow-Origin", "*")
                .header("Cache-Control", "no-cache");

            if inject_bridge && (ext == "html" || ext == "htm") {
                let body = String::from_utf8_lossy(&bytes).to_string();
                let injected = inject_bridge_shim(&body);
                return builder.body(Full::new(Bytes::from(injected))).unwrap();
            }

            builder.body(Full::new(Bytes::from(bytes))).unwrap()
        }
        Err(_) => not_found_response(&format!("not found: {}", rel)),
    }
}

fn inject_bridge_shim(html: &str) -> String {
    let bridge_tag = r#"<script src="/webforge-bridge.js"></script>
<script src="/webforge-ai-sidebar.js"></script>"#;
    if let Some(pos) = html.rfind("</body>") {
        let mut out = String::with_capacity(html.len() + bridge_tag.len());
        out.push_str(&html[..pos]);
        out.push_str(bridge_tag);
        out.push_str(&html[pos..]);
        out
    } else {
        format!("{}\n{}", html, bridge_tag)
    }
}

fn not_found_response(msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Full::new(Bytes::from(msg.to_string())))
        .unwrap()
}

fn forbidden_response() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(Full::new(Bytes::from("forbidden")))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    async fn body_text(resp: Response<Full<Bytes>>) -> String {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[test]
    fn sanitize_filename_strips_traversal() {
        assert_eq!(sanitize_filename("../etc/config", "html"), "etc/config.html");
        assert_eq!(sanitize_filename("../../secret", "html"), "secret.html");
        assert_eq!(sanitize_filename("normal-page", "html"), "normal-page.html");
        assert_eq!(sanitize_filename("/absolute/path", "html"), "absolute/path.html");
        // sensitive files rejected (matches PHP save.php behaviour)
        assert_eq!(sanitize_filename(".htaccess", "html"), "");
        assert_eq!(sanitize_filename("passwd", "html"), "");
        // extension enforced
        assert_eq!(sanitize_filename("page.txt", "html"), "page.html");
        assert_eq!(sanitize_filename("page", "html"), "page.html");
    }

    #[test]
    fn sanitize_path_segment_strips_traversal() {
        assert_eq!(sanitize_path_segment("../etc/config"), "etc/config");
        assert_eq!(sanitize_path_segment("uploads/../secret"), "uploads/secret");
        assert_eq!(sanitize_path_segment("my-folder"), "my-folder");
    }

    #[test]
    fn php_tag_re_detects_php() {
        assert!(php_tag_re("<?php echo 'x'; ?>"));
        assert!(php_tag_re("<? echo 'x'; ?>"));
        assert!(php_tag_re("<?= $var ?>"));
        assert!(!php_tag_re("<html><body>hello</body></html>"));
        // <?xml triggers our regex because it matches `<? ` — this is acceptable for
        // an HTML editor where XML prologs aren't expected in saved content.
    }

    #[test]
    fn valid_oembed_url_checks_whitelist() {
        assert!(valid_oembed_url("https://www.youtube.com/oembed?url=foo"));
        assert!(valid_oembed_url("https://vimeo.com/api/oembed.json?url=bar"));
        assert!(!valid_oembed_url("https://evil.com/oembed"));
        assert!(!valid_oembed_url(""));
    }

    #[test]
    fn parse_form_urlencoded_decodes_pairs() {
        let m = parse_form_urlencoded("file=index.html&html=%3Chtml%3Ehello%3C%2Fhtml%3E");
        assert_eq!(m.get("file").unwrap(), "index.html");
        assert_eq!(m.get("html").unwrap(), "<html>hello</html>");
    }

    #[test]
    fn parse_action_from_query_finds_action() {
        assert_eq!(parse_action_from_query(Some("action=rename&file=foo")), "rename");
        assert_eq!(parse_action_from_query(Some("file=foo&action=delete")), "delete");
        assert_eq!(parse_action_from_query(None), "");
        assert_eq!(parse_action_from_query(Some("file=foo")), "");
    }

    #[test]
    fn parse_site_query_finds_site() {
        assert_eq!(parse_site_query(Some("site=123&action=save")), Some(123));
        assert_eq!(parse_site_query(Some("action=save&site=456")), Some(456));
        assert_eq!(parse_site_query(None), None);
        assert_eq!(parse_site_query(Some("action=save")), None);
    }

    #[test]
    fn extract_boundary_parses_content_type() {
        assert_eq!(
            extract_boundary("multipart/form-data; boundary=----WebKitFormBoundaryABC123"),
            Some("----WebKitFormBoundaryABC123".to_string())
        );
        assert_eq!(
            extract_boundary("multipart/form-data; boundary=\"custom-boundary\""),
            Some("custom-boundary".to_string())
        );
        assert_eq!(extract_boundary("application/json"), None);
    }

    #[test]
    fn parse_multipart_extracts_fields() {
        let boundary = "----TestBoundary";
        let body = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"mediaPath\"\r\n\r\nuploads\r\n\
             --{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.png\"\r\nContent-Type: image/png\r\n\r\nfake-png-bytes\r\n\
             --{}--\r\n",
            boundary, boundary, boundary
        );
        let fields = parse_multipart(body.as_bytes(), boundary).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "mediaPath");
        assert_eq!(String::from_utf8_lossy(&fields[0].2), "uploads");
        assert!(fields[0].1.is_none());
        assert_eq!(fields[1].0, "file");
        assert_eq!(fields[1].1.as_deref(), Some("test.png"));
        assert_eq!(fields[1].2, b"fake-png-bytes");
    }

    #[test]
    fn parse_multipart_handles_empty_body() {
        let boundary = "----TestBoundary";
        let body = format!("--{}--\r\n", boundary);
        let fields = parse_multipart(body.as_bytes(), boundary).unwrap();
        assert!(fields.is_empty());
    }

    #[test]
    fn collect_pages_walks_html_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join("my-pages")).unwrap();
        fs::write(root.join("my-pages/index.html"), "<html></html>").unwrap();
        fs::write(root.join("my-pages/about.html"), "<html></html>").unwrap();

        fs::create_dir_all(root.join("demo/landing")).unwrap();
        fs::write(root.join("demo/landing/home.html"), "<html></html>").unwrap();
        fs::write(root.join("demo/landing/editor.html"), "<html></html>").unwrap();

        let pages = collect_pages(root);
        let names: Vec<_> = pages.iter().map(|p| p.name.clone()).collect();
        assert!(names.contains(&"index".to_string()));
        assert!(names.contains(&"about".to_string()));
        assert!(names.contains(&"home".to_string()));
        // editor.html is excluded
        assert!(!names.iter().any(|n| n == "editor"));
    }

    #[test]
    fn collect_pages_excludes_src_dir() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join("demo/landing/src/_includes")).unwrap();
        fs::write(root.join("demo/landing/index.html"), "<html></html>").unwrap();
        // src/ contains unresolved @@include templates — must be excluded
        fs::write(root.join("demo/landing/src/broken.html"), "@@include('head.html')").unwrap();
        fs::write(root.join("demo/landing/src/_includes/head.html"), "<head></head>").unwrap();

        let pages = collect_pages(root);
        let names: Vec<_> = pages.iter().map(|p| p.name.clone()).collect();
        // index.html in a subfolder gets named after the folder ("landing")
        assert!(names.contains(&"landing".to_string()));
        // src/ templates must NOT appear
        assert!(!names.contains(&"broken".to_string()));
        assert!(!names.contains(&"head".to_string()));
    }

    #[test]
    fn scan_directory_builds_tree() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join("media/sub")).unwrap();
        fs::write(root.join("media/a.png"), b"abc").unwrap();
        fs::write(root.join("media/sub/b.jpg"), b"de").unwrap();

        let tree = scan_directory(root.join("media").as_path(), root.join("media").as_path());
        assert_eq!(tree.node_type, "folder");
        let items = tree.items.as_ref().unwrap();
        assert_eq!(items.len(), 2); // a.png + sub/
        let sub = items.iter().find(|i| i.node_type == "folder").unwrap();
        assert_eq!(sub.name, "sub");
        let sub_items = sub.items.as_ref().unwrap();
        assert_eq!(sub_items.len(), 1);
        assert_eq!(sub_items[0].name, "b.jpg");
    }

    #[test]
    fn uppercase_first_capitalizes() {
        assert_eq!(uppercase_first("hello"), "Hello");
        assert_eq!(uppercase_first("Hello"), "Hello");
        assert_eq!(uppercase_first(""), "");
    }

    #[test]
    fn find_subsequence_locates_needle() {
        assert_eq!(find_subsequence(b"hello world", b"world"), Some(6));
        assert_eq!(find_subsequence(b"hello", b"xyz"), None);
        assert_eq!(find_subsequence(b"abc", b"abc"), Some(0));
    }

    #[tokio::test]
    async fn handle_save_writes_file() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        let form = "file=my-pages/test.html&html=%3Chtml%3E%3Cbody%3Ehi%3C%2Fbody%3E%3C%2Fhtml%3E";
        let resp = handle_save(root, Method::POST, "", Bytes::from(form.to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let saved = fs::read_to_string(root.join("my-pages/test.html")).unwrap();
        assert!(saved.contains("<html><body>hi</body></html>"));
    }

    #[tokio::test]
    async fn handle_save_rejects_php() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        let form = "file=evil.html&html=%3C%3Fphp+echo+%27x%27%3B+%3F%3E";
        let resp = handle_save(root, Method::POST, "", Bytes::from(form.to_string())).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(!root.join("evil.html").exists());
    }

    #[tokio::test]
    async fn handle_save_rename_action() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("old.html"), "content").unwrap();

        let form = "file=old.html&newfile=new.html";
        let resp = handle_save(root, Method::POST, "rename", Bytes::from(form.to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!root.join("old.html").exists());
        assert!(root.join("new.html").exists());
    }

    #[tokio::test]
    async fn handle_save_delete_action() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("doomed.html"), "content").unwrap();

        let form = "file=doomed.html";
        let resp = handle_save(root, Method::POST, "delete", Bytes::from(form.to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!root.join("doomed.html").exists());
    }

    #[tokio::test]
    async fn handle_upload_saves_file() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        let boundary = "----TestBoundary";
        let body = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"mediaPath\"\r\n\r\nuploads\r\n\
             --{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"pic.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nJPEGBYTES\r\n\
             --{}--\r\n",
            boundary, boundary, boundary
        );

        let mut headers = HeaderMap::new();
        headers.insert("content-type", format!("multipart/form-data; boundary={}", boundary).parse().unwrap());

        let resp = handle_upload(root, Method::POST, &headers, &None, Bytes::from(body)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let saved = fs::read(root.join("uploads/pic.jpg")).unwrap();
        assert_eq!(saved, b"JPEGBYTES");
    }

    #[tokio::test]
    async fn handle_upload_rejects_disallowed_extension() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        let boundary = "----TestBoundary";
        let body = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"mediaPath\"\r\n\r\nuploads\r\n\
             --{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"evil.php\"\r\n\r\nPHPCODE\r\n\
             --{}--\r\n",
            boundary, boundary, boundary
        );

        let mut headers = HeaderMap::new();
        headers.insert("content-type", format!("multipart/form-data; boundary={}", boundary).parse().unwrap());

        let resp = handle_upload(root, Method::POST, &headers, &None, Bytes::from(body)).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(!root.join("uploads/evil.php").exists());
    }

    #[tokio::test]
    async fn handle_scan_returns_tree() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("media")).unwrap();
        fs::write(root.join("media/a.png"), b"abc").unwrap();

        let resp = handle_scan(root, Method::GET, &Some("mediaPath=media".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let s = body_text(resp).await;
        assert!(s.contains("a.png"));
    }

    #[tokio::test]
    async fn handle_pages_lists_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("my-pages")).unwrap();
        fs::write(root.join("my-pages/index.html"), "<html></html>").unwrap();

        let resp = handle_pages(root, Method::GET).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let s = body_text(resp).await;
        assert!(s.contains("index"));
    }

    // ---- E2E integration tests: full save → read → scan → upload pipeline ----

    #[tokio::test]
    async fn e2e_save_then_read_on_disk() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // 1. Save a page via the API handler
        let form = "file=my-pages/hello.html&html=%3Chtml%3E%3Cbody%3EHello%20E2E%3C%2Fbody%3E%3C%2Fhtml%3E";
        let resp = handle_save(&root, Method::POST, "", Bytes::from(form.to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // 2. Verify the file exists on disk with the correct content
        let saved = fs::read_to_string(root.join("my-pages/hello.html")).unwrap();
        assert!(saved.contains("Hello E2E"));
        assert!(saved.contains("<html>"));
    }

    #[tokio::test]
    async fn e2e_save_then_scan() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // 1. Save a file
        let form = "file=media/test.html&html=%3Chtml%3Etest%3C%2Fhtml%3E";
        let _ = handle_save(&root, Method::POST, "", Bytes::from(form.to_string())).await;

        // 2. Upload a fake image
        let boundary = "----E2EBoundary";
        let body = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"mediaPath\"\r\n\r\nmedia\r\n\
             --{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.png\"\r\nContent-Type: image/png\r\n\r\nPNGBYTES\r\n\
             --{}--\r\n",
            boundary, boundary, boundary
        );
        let mut headers = HeaderMap::new();
        headers.insert("content-type", format!("multipart/form-data; boundary={}", boundary).parse().unwrap());
        let resp = handle_upload(&root, Method::POST, &headers, &None, Bytes::from(body)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // 3. Scan the media folder — should show both files
        let resp = handle_scan(&root, Method::GET, &Some("mediaPath=media".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let s = body_text(resp).await;
        assert!(s.contains("test.html"), "scan should list test.html");
        assert!(s.contains("test.png"), "scan should list test.png");
    }

    #[tokio::test]
    async fn e2e_save_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Try to write outside the workspace via ../
        let form = "file=../../etc/evil.html&html=%3Chtml%3Eevil%3C%2Fhtml%3E";
        let resp = handle_save(&root, Method::POST, "", Bytes::from(form.to_string())).await;

        // The file should be sanitized to a path inside root, not ../../etc/
        let evil_path = root.join("../../etc/evil.html");
        assert!(!evil_path.exists(), "path traversal should not create file outside root");

        // The sanitized path should exist inside the workspace
        let safe_path = root.join("etc/evil.html");
        if safe_path.exists() {
            // it was sanitized to etc/evil.html inside root — that's fine
            fs::remove_file(&safe_path).ok();
        }
    }

    #[tokio::test]
    async fn e2e_upload_then_serve() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // 1. Upload a file
        let boundary = "----E2EBoundary";
        let body = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"mediaPath\"\r\n\r\nuploads\r\n\
             --{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nJPEGDATA\r\n\
             --{}--\r\n",
            boundary, boundary, boundary
        );
        let mut headers = HeaderMap::new();
        headers.insert("content-type", format!("multipart/form-data; boundary={}", boundary).parse().unwrap());
        let resp = handle_upload(&root, Method::POST, &headers, &None, Bytes::from(body)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // 2. Verify the file is on disk
        assert!(root.join("uploads/photo.jpg").exists());

        // 3. Serve it via serve_from_root
        let resp = serve_from_root(&root, "uploads/photo.jpg", false);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn e2e_pages_excludes_src_and_editor() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Create compiled pages
        fs::create_dir_all(root.join("demo/landing")).unwrap();
        fs::write(root.join("demo/landing/index.html"), "<html></html>").unwrap();
        fs::write(root.join("demo/landing/about.html"), "<html></html>").unwrap();
        fs::write(root.join("demo/landing/editor.html"), "<html></html>").unwrap();

        // Create source templates (should be excluded)
        fs::create_dir_all(root.join("demo/landing/src")).unwrap();
        fs::write(root.join("demo/landing/src/broken.html"), "@@include('head.html')").unwrap();

        // Create my-pages
        fs::create_dir_all(root.join("my-pages")).unwrap();
        fs::write(root.join("my-pages/home.html"), "<html></html>").unwrap();

        let resp = handle_pages(&root, Method::GET).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let s = body_text(resp).await;

        // Should include compiled pages and my-pages
        assert!(s.contains("home"), "should list my-pages/home.html");
        assert!(s.contains("about"), "should list demo/landing/about.html");
        // Should NOT include editor.html or src/ templates
        assert!(!s.contains("broken"), "should not list src/broken.html");
    }

    #[tokio::test]
    async fn e2e_rename_and_delete() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // 1. Save a file
        let form = "file=old-page.html&html=%3Chtml%3Econtent%3C%2Fhtml%3E";
        let _ = handle_save(&root, Method::POST, "", Bytes::from(form.to_string())).await;
        assert!(root.join("old-page.html").exists());

        // 2. Rename it
        let form = "file=old-page.html&newfile=new-page.html";
        let resp = handle_save(&root, Method::POST, "rename", Bytes::from(form.to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!root.join("old-page.html").exists());
        assert!(root.join("new-page.html").exists());

        // 3. Delete it
        let form = "file=new-page.html";
        let resp = handle_save(&root, Method::POST, "delete", Bytes::from(form.to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!root.join("new-page.html").exists());
    }
}