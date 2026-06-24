use crate::folder::{read_file, FileEntry, FolderScan};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeneratedSection {
    pub key: String,
    pub name: String,
    pub source_file: String,
    pub html: String,
    pub tag: String,
    pub class: String,
    pub has_assets: bool,
    /// screenshot PNG bytes (set by generate_screenshots, empty until then)
    #[serde(default)]
    pub screenshot: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConversionResult {
    pub sections: Vec<GeneratedSection>,
    pub sections_js: String,
    pub group_name: String,
    pub asset_paths: Vec<String>,
    /// map of original asset path -> rewritten path (assets/... or absolute URL)
    #[serde(default)]
    pub asset_rewrites: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub fn convert_template(
    scan: FolderScan,
    group_name: String,
    kind: String,
    wp_url: Option<String>,
) -> Result<ConversionResult, String> {
    let group = if group_name.trim().is_empty() {
        "imported".to_string()
    } else {
        sanitize_key(&group_name)
    };

    let mut sections: Vec<GeneratedSection> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut asset_paths: BTreeSet<String> = BTreeSet::new();
    let mut asset_rewrites: Vec<(String, String)> = Vec::new();

    // ---- WP live render path: fetch rendered HTML from a running WP instance ----
    if kind == "wordpress" {
        if let Some(wp_base) = wp_url.as_ref() {
            if !wp_base.trim().is_empty() {
                log_warn(&mut warnings, &format!("Fetching rendered pages from WP instance at {}", wp_base));
                let pages = wp_fetch_pages(wp_base, &mut warnings)?;
                for page in pages {
                    let chunks = split_into_sections(&page.html);
                    if chunks.is_empty() {
                        warnings.push(format!("No sections in WP page {}", page.url));
                        continue;
                    }
                    for (idx, chunk) in chunks.iter().enumerate() {
                        let base_name = page.name.clone();
                        let key = if chunks.len() == 1 {
                            format!("{}/{}", group, sanitize_key(&base_name))
                        } else {
                            format!("{}/{}-{}", group, sanitize_key(&base_name), idx + 1)
                        };
                        let name = prettify(&base_name, idx, chunks.len());
                        // rewrite asset URLs in the rendered HTML to absolute (WP) URLs
                        let rewritten_html = rewrite_assets_to_absolute(&chunk.html, &page.url);
                        collect_asset_paths(&rewritten_html, &mut asset_paths);
                        sections.push(GeneratedSection {
                            key: key.clone(),
                            name: name.clone(),
                            source_file: page.url.clone(),
                            html: rewritten_html,
                            tag: chunk.tag.clone(),
                            class: chunk.class.clone(),
                            has_assets: chunk.has_assets,
                            screenshot: None,
                        });
                    }
                }
                if !sections.is_empty() {
                    // for WP, assets stay as absolute URLs (no rewriting to assets/)
                    for p in &asset_paths {
                        asset_rewrites.push((p.clone(), p.clone()));
                    }
                    let sections_js = render_sections_js(&group, &sections, &[]);
                    return Ok(ConversionResult {
                        sections,
                        sections_js,
                        group_name: group,
                        asset_paths: asset_paths.into_iter().collect(),
                        asset_rewrites,
                        warnings,
                    });
                }
                // if no sections from live render, fall through to PHP parsing
                warnings.push("Live render returned no sections, falling back to PHP parsing".to_string());
            }
        }
    }

    // ---- Static HTML / PHP-parsing path ----
    let files: Vec<FileEntry> = if kind == "wordpress" {
        let mut candidates: Vec<FileEntry> = scan
            .php_files
            .iter()
            .filter(|f| f.relative_path.to_lowercase().contains("template-parts"))
            .cloned()
            .collect();
        if candidates.is_empty() {
            candidates = scan
                .php_files
                .iter()
                .filter(|f| {
                    let name = Path::new(&f.relative_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    ["header.php", "footer.php", "single.php", "page.php", "index.php", "front-page.php", "archive.php"]
                        .iter()
                        .any(|t| name.eq_ignore_ascii_case(t))
                })
                .cloned()
                .collect();
        }
        if candidates.is_empty() {
            scan.php_files.clone()
        } else {
            candidates
        }
    } else {
        scan.html_files.clone()
    };

    if files.is_empty() {
        return Err("No template files found to convert".to_string());
    }

    for file in &files {
        let raw = read_file(&file.path)?;
        let html = if kind == "wordpress" {
            strip_php(&raw)
        } else {
            raw
        };

        let chunks = split_into_sections(&html);
        if chunks.is_empty() {
            warnings.push(format!(
                "No <section>/<header>/<footer>/<nav>/<aside> found in {}, skipping",
                file.relative_path
            ));
            continue;
        }

        for (idx, chunk) in chunks.iter().enumerate() {
            let base_name = Path::new(&file.relative_path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "section".to_string());
            let key = if chunks.len() == 1 {
                format!("{}/{}", group, sanitize_key(&base_name))
            } else {
                format!("{}/{}-{}", group, sanitize_key(&base_name), idx + 1)
            };
            let name = prettify(&base_name, idx, chunks.len());

            // rewrite asset paths to assets/... (relative to the zip root)
            let (rewritten_html, rewrites) = rewrite_assets_to_local(&chunk.html, &file.relative_path);
            for (orig, new) in &rewrites {
                asset_paths.insert(orig.clone());
                asset_rewrites.push((orig.clone(), new.clone()));
            }
            // also collect the original paths for the export to copy
            collect_asset_paths(&chunk.html, &mut asset_paths);

            sections.push(GeneratedSection {
                key: key.clone(),
                name: name.clone(),
                source_file: file.relative_path.clone(),
                html: rewritten_html,
                tag: chunk.tag.clone(),
                class: chunk.class.clone(),
                has_assets: chunk.has_assets,
                screenshot: None,
            });
        }
    }

    if sections.is_empty() {
        warnings.push("No sections were generated".to_string());
    }

    let sections_js = render_sections_js(&group, &sections, &[]);
    Ok(ConversionResult {
        sections,
        sections_js,
        group_name: group,
        asset_paths: asset_paths.into_iter().collect(),
        asset_rewrites,
        warnings,
    })
}

struct WpPage {
    name: String,
    url: String,
    html: String,
}

/// Fetch rendered pages from a running WordPress instance.
/// Tries the homepage and a few common page slugs.
fn wp_fetch_pages(wp_base: &str, warnings: &mut Vec<String>) -> Result<Vec<WpPage>, String> {
    let base = wp_base.trim_end_matches('/');
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("WebforgeStudio/0.1 ThemeBuilder")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // candidate pages to fetch: homepage + common WP pages
    let candidates = vec![
        ("home", format!("{}/", base)),
        ("front-page", format!("{}/", base)),
        ("about", format!("{}/about/", base)),
        ("contact", format!("{}/contact/", base)),
        ("blog", format!("{}/blog/", base)),
        ("sample-page", format!("{}/sample-page/", base)),
    ];

    let mut pages = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    for (name, url) in candidates {
        if seen_urls.contains(&url) {
            continue;
        }
        match client.get(&url).send() {
            Ok(res) => {
                let status = res.status();
                if !status.is_success() {
                    continue;
                }
                let final_url = res.url().to_string();
                let content_type = res
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                if !content_type.contains("text/html") {
                    continue;
                }
                let body = res.text().map_err(|e: reqwest::Error| e.to_string())?;
                let body_len = body.len();
                seen_urls.insert(final_url.clone());
                pages.push(WpPage {
                    name: name.to_string(),
                    url: final_url,
                    html: body,
                });
                log_warn(warnings, &format!("Fetched WP page: {} ({} bytes)", name, body_len));
            }
            Err(e) => {
                log_warn(warnings, &format!("Could not fetch {} ({}), skipping", url, e));
            }
        }
        if pages.len() >= 4 {
            break;
        }
    }

    if pages.is_empty() {
        warnings.push("No WP pages could be fetched. Check the WP URL is reachable.".to_string());
    }
    Ok(pages)
}

fn log_warn(warnings: &mut Vec<String>, msg: &str) {
    warnings.push(msg.to_string());
}

/// Rewrite asset URLs in HTML to point to assets/<relative path> inside the export zip.
/// Resolves relative paths against the source file's location.
fn rewrite_assets_to_local(html: &str, source_file: &str) -> (String, Vec<(String, String)>) {
    let source_dir = Path::new(source_file)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut rewrites = Vec::new();
    let mut out = html.to_string();

    let attrs = ["src=\"", "href=\"", "src='", "href='", "poster=\"", "data-src=\""];
    for attr in attrs.iter() {
        let quote = if attr.ends_with('\'') { '\'' } else { '"' };
        let mut search = 0;
        while let Some(p) = out[search..].to_lowercase().find(&attr.to_lowercase()) {
            let abs = search + p + attr.len();
            if let Some(end) = out[abs..].find(quote) {
                let val = &out[abs..abs + end];
                if should_rewrite(val) {
                    let resolved = resolve_relative(val, &source_dir);
                    let new_val = format!("assets/{}", resolved.trim_start_matches('/'));
                    rewrites.push((val.to_string(), new_val.clone()));
                    out.replace_range(abs..abs + end, &new_val);
                    search = abs + new_val.len();
                } else {
                    search = abs + end;
                }
            } else {
                search = abs;
            }
        }
    }
    (out, rewrites)
}

/// Rewrite asset URLs in HTML to absolute URLs (for WP/remote-rendered pages).
/// Resolves relative paths against the page's final URL.
fn rewrite_assets_to_absolute(html: &str, base_url: &str) -> String {
    let mut out = html.to_string();
    let attrs = ["src=\"", "href=\"", "src='", "href='", "poster=\"", "data-src=\""];
    for attr in attrs.iter() {
        let quote = if attr.ends_with('\'') { '\'' } else { '"' };
        let mut search = 0;
        while let Some(p) = out[search..].to_lowercase().find(&attr.to_lowercase()) {
            let abs = search + p + attr.len();
            if let Some(end) = out[abs..].find(quote) {
                let val = &out[abs..abs + end];
                if should_rewrite(val) {
                    if let Some(abs_url) = absolutize(val, base_url) {
                        out.replace_range(abs..abs + end, &abs_url);
                        search = abs + abs_url.len();
                    } else {
                        search = abs + end;
                    }
                } else {
                    search = abs + end;
                }
            } else {
                search = abs;
            }
        }
    }
    out
}

fn should_rewrite(val: &str) -> bool {
    !val.is_empty()
        && !val.starts_with("http://")
        && !val.starts_with("https://")
        && !val.starts_with("//")
        && !val.starts_with("data:")
        && !val.starts_with("#")
        && !val.starts_with("<!--")
        && !val.starts_with("mailto:")
        && !val.starts_with("javascript:")
        && !val.starts_with("tel:")
        && (val.contains('.') || val.contains('/'))
}

fn resolve_relative(val: &str, source_dir: &str) -> String {
    let val = val.trim_start_matches("./");
    if val.starts_with('/') {
        return val.trim_start_matches('/').to_string();
    }
    if source_dir.is_empty() {
        return val.to_string();
    }
    // join source_dir + val, normalize ../
    let combined = format!("{}/{}", source_dir, val);
    normalize_path(&combined)
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    parts.join("/")
}

fn absolutize(val: &str, base_url: &str) -> Option<String> {
    if let Ok(base) = url::Url::parse(base_url) {
        if let Ok(joined) = base.join(val) {
            let scheme = joined.scheme();
            if scheme == "http" || scheme == "https" {
                return Some(joined.to_string());
            }
        }
    }
    None
}

struct Chunk {
    html: String,
    tag: String,
    class: String,
    has_assets: bool,
}

fn split_into_sections(html: &str) -> Vec<Chunk> {
    let target_tags = ["section", "header", "footer", "nav", "aside"];
    let mut chunks: Vec<(usize, usize, Chunk)> = Vec::new();
    for tag in target_tags.iter() {
        let mut search_from = 0;
        loop {
            let open = match find_open_tag(html, tag, search_from) {
                Some(i) => i,
                None => break,
            };
            let (close, _end_close) = match find_close_tag(html, tag, open) {
                Some(p) => p,
                None => break,
            };
            if chunks.iter().any(|(s, e, _)| *s <= open && close <= *e) {
                search_from = close;
                continue;
            }
            let chunk_html = &html[open..close];
            let (class, has_assets) = extract_meta(chunk_html);
            chunks.push((
                open,
                close,
                Chunk {
                    html: chunk_html.to_string(),
                    tag: tag.to_string(),
                    class,
                    has_assets,
                },
            ));
            search_from = close;
        }
    }
    chunks.sort_by_key(|(s, _, _)| *s);
    chunks.into_iter().map(|(_, _, c)| c).collect()
}

fn find_open_tag(html: &str, tag: &str, from: usize) -> Option<usize> {
    let lower = html.to_lowercase();
    let needle = format!("<{}", tag);
    lower[from..].find(&needle).map(|i| from + i)
}

fn find_close_tag(html: &str, tag: &str, open_pos: usize) -> Option<(usize, usize)> {
    let lower = html.to_lowercase();
    let open_tag = format!("<{}", tag);
    let close_tag = format!("</{}>", tag);
    let mut depth = 0;
    let mut pos = open_pos;
    while let Some(next) = lower[pos..].find(&open_tag) {
        let abs = pos + next;
        let after = lower.get(abs + open_tag.len()..).unwrap_or("");
        if after.starts_with(' ') || after.starts_with('>') || after.starts_with('/') {
            depth += 1;
            pos = abs + open_tag.len();
        } else {
            pos = abs + open_tag.len();
        }
        if let Some(close_idx) = lower[pos..].find(&close_tag) {
            let abs_close = pos + close_idx;
            depth -= 1;
            if depth == 0 {
                let end = abs_close + close_tag.len();
                return Some((abs_close, end));
            }
            pos = abs_close + close_tag.len();
        } else {
            return None;
        }
    }
    None
}

fn extract_meta(html: &str) -> (String, bool) {
    let lower = html.to_lowercase();
    let class = if let Some(class_pos) = lower.find("class=\"") {
        let start = class_pos + 7;
        if let Some(end) = html[start..].find('"') {
            html[start..start + end].split_whitespace().next().unwrap_or("").to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let has_assets = lower.contains("<img") || lower.contains("background-image") || lower.contains("<video");
    (class, has_assets)
}

fn collect_asset_paths(html: &str, out: &mut BTreeSet<String>) {
    let mut collect = |attr: &str, prefix: &str| {
        let mut search = 0;
        while let Some(p) = html[search..].to_lowercase().find(attr) {
            let abs = search + p + attr.len();
            let rest = &html[abs..];
            if let Some(quote_end) = rest.find(prefix) {
                let val_start = abs + quote_end + prefix.len();
                if let Some(end) = html[val_start..].find(|c: char| c == '"' || c == '\'') {
                    let val = &html[val_start..val_start + end];
                    let is_real_asset = !val.is_empty()
                        && !val.starts_with("http")
                        && !val.starts_with("//")
                        && !val.starts_with("data:")
                        && !val.starts_with("#")
                        && !val.starts_with("<!--")
                        && !val.starts_with("mailto:")
                        && !val.starts_with("javascript:")
                        && !val.starts_with("tel:")
                        && (val.contains('.') || val.contains('/'));
                    if is_real_asset {
                        out.insert(val.to_string());
                    }
                    search = val_start + end;
                    continue;
                }
            }
            search = abs;
        }
    };
    collect("src=\"", "");
    collect("src='", "");
    collect("href=\"", "");
    collect("href='", "");
}

fn strip_php(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();
    let bytes = input.as_bytes();
    while let Some((i, c)) = chars.next() {
        if c == '<' && input[i..].starts_with("<?php") {
            if let Some(end) = find_case_insensitive(&input[i..], "?>") {
                let _ = bytes;
                out.push_str("<!-- php -->");
                let skip_chars = input[i..i + end + 2].chars().count();
                for _ in 0..skip_chars {
                    chars.next();
                }
            } else {
                break;
            }
        } else if c == '<' && input[i..].starts_with("<?=") {
            if let Some(end) = find_case_insensitive(&input[i..], "?>") {
                out.push_str("<!-- php-echo -->");
                let skip_chars = input[i..i + end + 2].chars().count();
                for _ in 0..skip_chars {
                    chars.next();
                }
            } else {
                break;
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack.to_lowercase().find(&needle.to_lowercase())
}

fn sanitize_key(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn prettify(base: &str, idx: usize, total: usize) -> String {
    let title = base
        .split(|c: char| c == '-' || c == '_')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if total == 1 {
        title
    } else {
        format!("{} {}", title, idx + 1)
    }
}

/// Render sections.js. If screenshots are provided (key -> filename in zip),
/// the image: field points to the screenshot; otherwise it's empty.
fn render_sections_js(group: &str, sections: &[GeneratedSection], screenshots: &[(String, String)]) -> String {
    let mut out = String::new();
    out.push_str("/* Webforge Studio - generated sections */\n\n");
    out.push_str(&format!("Webforge.SectionsGroup['{}'] = [\n", group));
    for s in sections {
        out.push_str(&format!("  \"{}\",\n", s.key));
    }
    out.push_str("];\n\n");
    for s in sections {
        let escaped_html = escape_backticks(&s.html);
        let image = screenshots
            .iter()
            .find(|(k, _)| k == &s.key)
            .map(|(_, f)| f.clone())
            .unwrap_or_default();
        out.push_str(&format!(
            "Webforge.Sections.add(\"{}\", {{\n  name: \"{}\",\n  image: \"{}\",\n  html: `{}`\n}});\n\n",
            s.key, escape_double_quotes(&s.name), image, escaped_html
        ));
    }
    out
}

/// Re-render sections.js with screenshot paths filled in (called after screenshots are generated)
#[tauri::command]
pub fn render_with_screenshots(
    group_name: String,
    sections: Vec<GeneratedSection>,
    screenshots: Vec<(String, String)>,
) -> Result<String, String> {
    Ok(render_sections_js(&group_name, &sections, &screenshots))
}

fn escape_backticks(s: &str) -> String {
    s.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${")
}

fn escape_double_quotes(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}