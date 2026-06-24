use crate::folder::{read_file, FileEntry, FolderScan};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tauri::{AppHandle, Emitter};

// ---- provider settings ----

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsInput {
    pub provider: String,
    pub baseUrl: String,
    pub apiKey: String,
    pub model: String,
}

// ---- LLM API types (OpenAI-compatible) ----

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

// ---- AI conversion result ----

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AiSection {
    pub key: String,
    pub name: String,
    pub tag: String,
    pub html: String,
    pub source_file: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AiConversionResult {
    pub sections: Vec<AiSection>,
    pub sections_js: String,
    pub group_name: String,
    pub asset_paths: Vec<String>,
    pub warnings: Vec<String>,
    pub reasoning: String,
}

// ---- progress events ----

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiProgress {
    pub step: String,
    pub message: String,
    pub percent: u8,
}

// ---- provider presets ----

pub fn provider_presets() -> Vec<(String, String, String)> {
    // (provider name, default base_url, default model)
    vec![
        ("ollama-cloud".to_string(), "https://api.ovhcloud.com/ollama/v1".to_string(), "llama3.1".to_string()),
        ("ollama-local".to_string(), "http://localhost:11434/v1".to_string(), "llama3.1".to_string()),
        ("openai".to_string(), "https://api.openai.com/v1".to_string(), "gpt-4o".to_string()),
        ("groq".to_string(), "https://api.groq.com/openai/v1".to_string(), "llama-3.3-70b-versatile".to_string()),
        ("together".to_string(), "https://api.together.xyz/v1".to_string(), "meta-llama/Llama-3.3-70B-Instruct-Turbo".to_string()),
        ("openrouter".to_string(), "https://openrouter.ai/api/v1".to_string(), "anthropic/claude-3.5-sonnet".to_string()),
        ("custom".to_string(), "".to_string(), "".to_string()),
    ]
}

#[tauri::command]
pub fn get_provider_presets() -> Vec<(String, String, String)> {
    provider_presets()
}

// ---- save / load settings ----

#[tauri::command]
pub fn save_ai_settings(settings: AiSettingsInput) -> Result<(), String> {
    // store in a JSON file in the app data dir (simple, no encryption needed for dev;
    // for production, use tauri-plugin-stronghold or OS keychain)
    let dir = dirs_next::config_dir()
        .ok_or("Cannot find config directory")?
        .join("com.webforge.studio");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("ai-settings.json");
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn load_ai_settings() -> Result<Option<AiSettingsInput>, String> {
    let path = dirs_next::config_dir()
        .ok_or("Cannot find config directory")?
        .join("com.webforge.studio")
        .join("ai-settings.json");
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let settings: AiSettingsInput = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    Ok(Some(settings))
}

// ---- the main AI conversion command ----

#[tauri::command]
pub fn convert_with_ai(
    scan: FolderScan,
    group_name: String,
    kind: String,
    settings: AiSettingsInput,
    app: AppHandle,
) -> Result<AiConversionResult, String> {
    let group = if group_name.trim().is_empty() {
        "imported".to_string()
    } else {
        sanitize_key(&group_name)
    };

    let mut warnings = Vec::new();
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    // ---- step 1: read and prepare template files ----
    emit_progress(&app, "reading", "Reading template files...", 5);

    let files: Vec<FileEntry> = if kind == "wordpress" {
        scan.php_files.iter().take(10).cloned().collect::<Vec<_>>()
    } else {
        scan.html_files.iter().take(10).cloned().collect::<Vec<_>>()
    };

    if files.is_empty() {
        return Err("No template files found".to_string());
    }

    // prepare file contents (truncate large files to fit context)
    let mut file_contents = Vec::new();
    for f in &files {
        let content = read_file(&f.path)?;
        let truncated = if content.len() > 8000 {
            format!("{}...[truncated, {} total chars]", &content[..8000], content.len())
        } else {
            content
        };
        file_contents.push((f.relative_path.clone(), truncated));
    }

    let total_files = file_contents.len();
    emit_progress(&app, "reading", &format!("Read {} template files", total_files), 10);

    // ---- step 2: build the prompt ----
    emit_progress(&app, "prompting", "Building conversion prompt...", 15);

    let system_prompt = build_system_prompt(&kind);
    let user_prompt = build_user_prompt(&file_contents, &group, &kind);

    emit_progress(&app, "calling-llm", "Sending to AI model (this may take 30-60s)...", 20);

    // ---- step 3: call the LLM ----
    let req = ChatRequest {
        model: settings.model.clone(),
        messages: vec![
            ChatMessage { role: "system".to_string(), content: system_prompt },
            ChatMessage { role: "user".to_string(), content: user_prompt },
        ],
        temperature: 0.3,
        stream: false,
    };

    let url = format!("{}/chat/completions", settings.baseUrl.trim_end_matches('/'));
    let res = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", settings.apiKey))
        .header("Content-Type", "application/json")
        .json(&req)
        .send();

    let res = tauri::async_runtime::block_on(res).map_err(|e| {
        format!("HTTP request to AI provider failed: {}. Check base URL and API key.", e)
    })?;

    let status = res.status();
    if !status.is_success() {
        let body = tauri::async_runtime::block_on(res.text()).unwrap_or_default();
        return Err(format!("AI provider returned error ({}): {}", status, body));
    }

    emit_progress(&app, "parsing", "Received response, parsing sections...", 80);

    let chat_res: ChatResponse = tauri::async_runtime::block_on(res.json())
        .map_err(|e| format!("Failed to parse AI response: {}", e))?;

    let content = chat_res
        .choices
        .first()
        .ok_or("AI returned no choices")?
        .message
        .content
        .clone();

    // ---- step 4: parse the LLM response into sections ----
    emit_progress(&app, "extracting", "Extracting sections from AI response...", 90);

    let (sections, reasoning) = parse_ai_response(&content, &group, &kind, &mut warnings)?;

    if sections.is_empty() {
        warnings.push("AI did not return any valid sections. Check the model output.".to_string());
        return Err("AI returned no sections".to_string());
    }

    // collect asset paths
    let mut asset_paths: BTreeSet<String> = BTreeSet::new();
    for s in &sections {
        collect_asset_paths(&s.html, &mut asset_paths);
    }

    // render sections.js
    let sections_js = render_sections_js(&group, &sections);

    emit_progress(&app, "done", &format!("Generated {} sections", sections.len()), 100);

    Ok(AiConversionResult {
        sections,
        sections_js,
        group_name: group,
        asset_paths: asset_paths.into_iter().collect(),
        warnings,
        reasoning,
    })
}

fn emit_progress(app: &AppHandle, step: &str, message: &str, percent: u8) {
    let _ = app.emit(
        "ai-progress",
        AiProgress {
            step: step.to_string(),
            message: message.to_string(),
            percent,
        },
    );
}

// ---- prompt construction ----

fn build_system_prompt(kind: &str) -> String {
    let wp_extra = if kind == "wordpress" {
        "\n\nThis is a WordPress theme. For PHP code blocks:\n\
- Replace `<?php wp_nav_menu(...) ?>` with a realistic `<ul class=\"menu\">...</ul>` with sample menu items\n\
- Replace `<?php bloginfo('name') ?>` with \"Site Name\" placeholder text\n\
- Replace `<?php bloginfo('description') ?>` with \"Site tagline\" placeholder text\n\
- Replace `<?php the_title() ?>` with \"Sample Page Title\"\n\
- Replace `<?php the_content() ?>` with 2-3 paragraphs of Lorem Ipsum\n\
- Replace `<?php echo esc_url(home_url('/')) ?>` with \"#\"\n\
- Replace `<?php get_header() ?>` / `<?php get_footer() ?>` with the actual header/footer HTML\n\
- Replace `<?php get_template_part('template-parts/hero') ?>` with the content of that template part\n\
- For any other `<?php ... ?>`, replace with appropriate placeholder HTML or empty string\n\
- Remove PHP comments `/** ... */` and `// ...`"
    } else {
        ""
    };

    format!(
        "You are an expert web developer specializing in converting HTML templates into reusable page builder sections.\n\
You analyze HTML templates and split them into clean, self-contained sections suitable for a drag-and-drop page builder.\n\n\
Your output MUST be valid JSON with this exact structure:\n\
```\n{{\n  \"reasoning\": \"Brief explanation of your analysis\",\n  \"sections\": [\n    {{\n      \"name\": \"Human-readable name (e.g., Hero, Features Grid, Pricing Table, Testimonials)\",\n      \"tag\": \"section|header|footer|nav|aside\",\n      \"html\": \"The complete, cleaned HTML for this section\"\n    }}\n  ]\n}}\n```\n\n\
Rules:\n\
1. Split the template into logical sections based on semantic meaning (hero, features, testimonials, footer, etc.)\n\
2. Each section's HTML must be self-contained and valid (proper open/close tags, no broken nesting)\n\
3. Clean up the HTML: remove unnecessary wrapper divs, fix malformed tags, normalize whitespace\n\
4. Generate descriptive names (not filenames) - \"Hero Section\", \"Features Grid\", \"Pricing Table\", not \"index-1\"\n\
5. Preserve all CSS classes, inline styles, and asset references (src, href) as-is\n\
6. Do NOT include `<!doctype>`, `<html>`, `<head>`, `<body>` tags - only the section content\n\
7. If a section contains nested `<section>`/`<header>`/`<footer>`/`<nav>` tags, keep them as part of the parent section\n\
8. Return between 2-12 sections depending on the template complexity\n\
9. The HTML must be a string in the JSON (properly escaped quotes, newlines as \\n){wp_extra}\n\n\
Return ONLY the JSON object, no markdown formatting, no explanation outside the JSON."
    )
}

fn build_user_prompt(files: &[(String, String)], group: &str, kind: &str) -> String {
    let mut prompt = format!(
        "Convert the following {} template files into Webforge page builder sections.\n\
Group name: \"{}\"\n\n",
        if kind == "wordpress" { "WordPress theme" } else { "HTML" },
        group
    );

    for (path, content) in files {
        prompt.push_str(&format!("--- File: {} ---\n{}\n\n", path, content));
    }

    prompt.push_str(&format!(
        "\nAnalyze these {} files and return the JSON with the sections array.",
        files.len()
    ));

    prompt
}

// ---- response parsing ----

fn parse_ai_response(
    content: &str,
    group: &str,
    _kind: &str,
    warnings: &mut Vec<String>,
) -> Result<(Vec<AiSection>, String), String> {
    // try to extract JSON from the response (LLMs sometimes wrap in ```json ... ```)
    let json_str = extract_json(content).ok_or_else(|| {
        format!("Could not find JSON in AI response. First 200 chars: {}", &content[..content.len().min(200)])
    })?;

    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| format!("Invalid JSON from AI: {}. First 200 chars: {}", e, &json_str[..json_str.len().min(200)]))?;

    let reasoning = parsed
        .get("reasoning")
        .and_then(|v| v.as_str())
        .unwrap_or("No reasoning provided")
        .to_string();

    let sections_arr = parsed
        .get("sections")
        .and_then(|v| v.as_array())
        .ok_or("AI response missing 'sections' array")?;

    let mut sections = Vec::new();
    for (idx, sec) in sections_arr.iter().enumerate() {
        let name = sec
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Section")
            .to_string();
        let tag = sec
            .get("tag")
            .and_then(|v| v.as_str())
            .unwrap_or("section")
            .to_string();
        let html = sec
            .get("html")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if html.is_empty() {
            warnings.push(format!("Section '{}' has empty HTML, skipping", name));
            continue;
        }

        let key = format!("{}/{}", group, sanitize_key(&name));

        sections.push(AiSection {
            key,
            name,
            tag,
            html,
            source_file: "ai-generated".to_string(),
        });
    }

    Ok((sections, reasoning))
}

fn extract_json(content: &str) -> Option<String> {
    // try direct parse first
    if serde_json::from_str::<serde_json::Value>(content).is_ok() {
        return Some(content.trim().to_string());
    }
    // try ```json ... ``` block
    if let Some(start) = content.find("```json") {
        let after = &content[start + 7..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    // try ``` ... ``` block (without json label)
    if let Some(start) = content.find("```") {
        let after = &content[start + 3..];
        // skip language label line if present
        let after = if after.starts_with("json\n") || after.starts_with("json\r") {
            &after[4..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    // try to find the first { and last }
    if let Some(start) = content.find('{') {
        if let Some(end) = content.rfind('}') {
            if end > start {
                return Some(content[start..=end].to_string());
            }
        }
    }
    None
}

fn collect_asset_paths(html: &str, out: &mut BTreeSet<String>) {
    let mut collect = |attr: &str| {
        let mut search = 0;
        while let Some(p) = html[search..].to_lowercase().find(attr) {
            let abs = search + p + attr.len();
            if let Some(end) = html[abs..].find('"').or_else(|| html[abs..].find('\'')) {
                let val = &html[abs..abs + end];
                if !val.is_empty()
                    && !val.starts_with("http")
                    && !val.starts_with("//")
                    && !val.starts_with("data:")
                    && !val.starts_with("#")
                    && !val.starts_with("mailto:")
                    && !val.starts_with("javascript:")
                    && !val.starts_with("tel:")
                    && (val.contains('.') || val.contains('/'))
                {
                    out.insert(val.to_string());
                }
                search = abs + end;
            } else {
                search = abs;
            }
        }
    };
    collect("src=\"");
    collect("href=\"");
}

fn render_sections_js(group: &str, sections: &[AiSection]) -> String {
    let mut out = String::new();
    out.push_str("/* Webforge Studio - AI-generated sections */\n\n");
    out.push_str(&format!("Webforge.SectionsGroup['{}'] = [\n", group));
    for s in sections {
        out.push_str(&format!("  \"{}\",\n", s.key));
    }
    out.push_str("];\n\n");
    for s in sections {
        let escaped_html = s.html.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${");
        let escaped_name = s.name.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(
            "Webforge.Sections.add(\"{}\", {{\n  name: \"{}\",\n  image: \"\",\n  html: `{}`\n}});\n\n",
            s.key, escaped_name, escaped_html
        ));
    }
    out
}

fn sanitize_key(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}