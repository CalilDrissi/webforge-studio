use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::{oneshot, Mutex};

// ---- JSON-RPC types ----

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

// ---- Tool definitions ----

#[derive(Debug, Serialize)]
struct ToolDef {
    name: &'static str,
    description: &'static str,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

pub fn tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef { name: "open_page", description: "Open a Webforge editor page by URL.", input_schema: json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}) },
        ToolDef { name: "get_html", description: "Get the current HTML of the page being edited.", input_schema: json!({"type":"object","properties":{"keep_helper_attributes":{"type":"boolean","default":false}}}) },
        ToolDef { name: "set_html", description: "Replace the entire page HTML in the editor.", input_schema: json!({"type":"object","properties":{"html":{"type":"string"}},"required":["html"]}) },
        ToolDef { name: "add_component", description: "Add a component/section/block to the canvas.", input_schema: json!({"type":"object","properties":{"component_type":{"type":"string"},"kind":{"type":"string","enum":["component","section","block"],"default":"section"}},"required":["component_type"]}) },
        ToolDef { name: "select_element", description: "Select an element in the editor by CSS selector.", input_schema: json!({"type":"object","properties":{"selector":{"type":"string"}},"required":["selector"]}) },
        ToolDef { name: "get_selected_element", description: "Get info about the currently selected element.", input_schema: json!({"type":"object","properties":{}}) },
        ToolDef { name: "set_property", description: "Set a property (attribute, style, text, html) on the selected element.", input_schema: json!({"type":"object","properties":{"property_type":{"type":"string","enum":["attribute","style","text","html"]},"name":{"type":"string"},"value":{"type":"string"}},"required":["property_type","name","value"]}) },
        ToolDef { name: "delete_element", description: "Delete the currently selected element.", input_schema: json!({"type":"object","properties":{}}) },
        ToolDef { name: "get_components", description: "List all available components, sections, and blocks.", input_schema: json!({"type":"object","properties":{}}) },
        ToolDef { name: "undo", description: "Undo the last change.", input_schema: json!({"type":"object","properties":{}}) },
        ToolDef { name: "redo", description: "Redo the last undone change.", input_schema: json!({"type":"object","properties":{}}) },
        ToolDef { name: "save_template", description: "Save the current page HTML.", input_schema: json!({"type":"object","properties":{"file":{"type":"string"}}}) },
    ]
}

// ---- Eval in editor via Tauri events ----

static PENDING_CALLS: tokio::sync::OnceCell<Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>> = tokio::sync::OnceCell::const_new();

async fn pending_calls() -> &'static Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> {
    PENDING_CALLS.get_or_init(|| async { Arc::new(Mutex::new(HashMap::new())) }).await
}

/// Called by the Tauri event listener when a `mcp-eval-result` event arrives.
/// Routes the result to the waiting oneshot channel.
pub async fn handle_eval_result(payload: Value) {
    let call_id = payload.get("callId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let pending = pending_calls().await;
    if let Some(tx) = pending.lock().await.remove(&call_id) {
        let _ = tx.send(payload);
    }
}

/// Execute a JS function in the active editor webview and wait for the result.
async fn eval_in_editor(app: &AppHandle, fn_body: &str, arg: Option<Value>) -> Result<Value, String> {
    let window = get_active_editor_window(app)?;
    let call_id = format!("mcp-{}", uuid_like());
    let (tx, rx) = oneshot::channel::<Value>();
    let pending = pending_calls().await;
    pending.lock().await.insert(call_id.clone(), tx);

    let arg_json = arg.map(|a| serde_json::to_string(&a).unwrap_or_else(|_| "null".to_string())).unwrap_or_else(|| "null".to_string());
    let js = format!(
        r#"
        (async function() {{
            try {{
                var f = new Function("return ({fn_body})")();
                var arg = {arg_json};
                var result = arg !== null ? await f(arg) : await f();
                window.__TAURI__.event.emit('mcp-eval-result', JSON.stringify({{ callId: '{call_id}', result: result }}));
            }} catch(e) {{
                window.__TAURI__.event.emit('mcp-eval-result', JSON.stringify({{ callId: '{call_id}', error: e.message || String(e) }}));
            }}
        }})();
        "#,
        fn_body = fn_body, arg_json = arg_json, call_id = call_id,
    );

    window.eval(&js).map_err(|e| format!("eval failed: {}", e))?;

    let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx).await;
    pending.lock().await.remove(&call_id);

    match result {
        Ok(Ok(val)) => {
            if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                Err(err.to_string())
            } else {
                Ok(val.get("result").cloned().unwrap_or(Value::Null))
            }
        }
        Ok(Err(_)) => Err("eval response channel closed".to_string()),
        Err(_) => Err("eval timed out (30s)".to_string()),
    }
}

fn get_active_editor_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    for (label, win) in app.webview_windows().iter() {
        if label.starts_with("workspace-") {
            return Ok(win.clone());
        }
    }
    Err("No editor window open. Open a workspace first.".to_string())
}

fn uuid_like() -> String {
    use std::time::SystemTime;
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}", nanos)
}

// ---- Tool dispatch ----

async fn dispatch_tool(app: &AppHandle, tool_name: &str, args: &Value) -> Result<Value, String> {
    let (fn_body, arg): (&str, Option<Value>) = match tool_name {
        "open_page" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            return Ok(json!({"content":[{"type":"text","text":format!("To open a page, use the Webforge Studio app. Editor URL: {}", url)}]}).into());
        }
        "get_html" => {
            let keep = args.get("keep_helper_attributes").and_then(|v| v.as_bool()).unwrap_or(false);
            (r#"(arg) => { try { return Webforge.Builder.getHtml(arg.keep) || ""; } catch(e) { return "Error: " + e.message; } }"#, Some(json!({"keep":keep})))
        }
        "set_html" => {
            let html = args.get("html").and_then(|v| v.as_str()).unwrap_or("").to_string();
            (r#"(html) => { Webforge.Builder.setHtml(html); Webforge.Undo.reset(); return "HTML set (" + html.length + " chars)"; }"#, Some(json!(html)))
        }
        "add_component" => {
            let ct = args.get("component_type").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("section").to_string();
            (r#"(args) => { try { var data; if(args.kind==='section')data=Webforge.Sections.get(args.component_type);else if(args.kind==='block')data=Webforge.Blocks.get(args.component_type);else data=Webforge.Components.get(args.component_type);if(!data)return JSON.stringify({error:"Component '"+args.component_type+"' not found"});var html=data.html||data.dragHtml||'';if(!html)return JSON.stringify({error:"Component has no html"});var doc=Webforge.Builder.frameDoc;if(!doc)return JSON.stringify({error:"Editor iframe not loaded"});var target=Webforge.Builder.selectedEl||doc.body;var wrapper=doc.createElement('div');wrapper.innerHTML=html;var node=wrapper.firstElementChild;if(node){if(args.kind==='section'||args.kind==='block')doc.body.appendChild(node);else if(target&&target!==doc.body)target.parentNode.insertBefore(node,target.nextSibling);else doc.body.appendChild(node);Webforge.Undo.addMutation({type:'childList',target:node.parentNode,addedNodes:[node]});Webforge.Builder.selectNode(node);if(Webforge.TreeList)Webforge.TreeList.loadComponents();}return JSON.stringify({success:true,message:"Added "+args.component_type,tag:node?node.tagName:''});}catch(e){return JSON.stringify({error:e.message});} }"#, Some(json!({"component_type":ct,"kind":kind})))
        }
        "select_element" => {
            let sel = args.get("selector").and_then(|v| v.as_str()).unwrap_or("").to_string();
            (r#"(sel) => { try { var doc=Webforge.Builder.frameDoc;if(!doc)return JSON.stringify({error:"Editor iframe not loaded"});var el=doc.querySelector(sel);if(!el)return JSON.stringify({error:"No element found: "+sel});Webforge.Builder.selectNode(el);return JSON.stringify({success:true,tagName:el.tagName,className:el.className,id:el.id});}catch(e){return JSON.stringify({error:e.message});} }"#, Some(json!(sel)))
        }
        "get_selected_element" => {
            (r#"() => { var el=Webforge.Builder.selectedEl;if(!el)return JSON.stringify({error:"No element selected"});return JSON.stringify({tagName:el.tagName,id:el.id||null,className:el.className||null,attributes:Array.from(el.attributes).map(a=>({name:a.name,value:a.value})),innerHTML:el.innerHTML.length>500?el.innerHTML.slice(0,500)+"...":el.innerHTML,childCount:el.children.length}); }"#, None)
        }
        "set_property" => {
            let pt = args.get("property_type").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
            (r#"(args) => { try { var el=Webforge.Builder.selectedEl;if(!el)return JSON.stringify({error:"No element selected"});if(args.property_type==='attribute'){el.setAttribute(args.name,args.value);Webforge.Undo.addMutation({type:'attributes',target:el,attributeName:args.name});}else if(args.property_type==='style'){el.style[args.name]=args.value;Webforge.Undo.addMutation({type:'style',target:el,attributeName:args.name});}else if(args.property_type==='text'){el.textContent=args.value;Webforge.Undo.addMutation({type:'characterData',target:el});}else if(args.property_type==='html'){el.innerHTML=args.value;Webforge.Undo.addMutation({type:'childList',target:el});}Webforge.Builder.reloadComponent();return JSON.stringify({success:true,message:"Set "+args.property_type+" '"+args.name+"'"});}catch(e){return JSON.stringify({error:e.message});} }"#, Some(json!({"property_type":pt,"name":name,"value":value})))
        }
        "delete_element" => {
            (r#"() => { try { var el=Webforge.Builder.selectedEl;if(!el)return JSON.stringify({error:"No element selected"});var parent=el.parentNode;if(!parent)return JSON.stringify({error:"Cannot delete root"});Webforge.Undo.addMutation({type:'childList',target:parent,removedNodes:[el]});el.remove();Webforge.Builder.selectNode(null);if(Webforge.TreeList)Webforge.TreeList.loadComponents();return JSON.stringify({success:true,message:"Deleted <"+el.tagName+">"});}catch(e){return JSON.stringify({error:e.message});} }"#, None)
        }
        "get_components" => {
            (r#"() => { var out={components:[],sections:[],blocks:[]};for(var g in Webforge.ComponentsGroup){for(var t of Webforge.ComponentsGroup[g]){var c=Webforge.Components.get(t);out.components.push({type:t,name:c?c.name:t,group:g});}}for(var g in Webforge.SectionsGroup){for(var t of Webforge.SectionsGroup[g]){var s=Webforge.Sections.get(t);out.sections.push({type:t,name:s?s.name:t,group:g});}}for(var g in Webforge.BlocksGroup){for(var t of Webforge.BlocksGroup[g]){var b=Webforge.Blocks.get(t);out.blocks.push({type:t,name:b?b.name:t,group:g});}}return JSON.stringify(out); }"#, None)
        }
        "undo" => ("() => { Webforge.Undo.undo(); return 'Undo executed.'; }", None),
        "redo" => ("() => { Webforge.Undo.redo(); return 'Redo executed.'; }", None),
        "save_template" => {
            let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("").to_string();
            (r#"(args) => { try { var btn=document.querySelector('.save-btn')||document.querySelector('[data-webforge-action="saveAjax"]');if(btn)btn.click();return JSON.stringify({success:true,message:"Save triggered",file:args.file||""});}catch(e){return JSON.stringify({error:e.message});} }"#, Some(json!({"file":file})))
        }
        _ => return Err(format!("Unknown tool: {}", tool_name)),
    };

    let result = eval_in_editor(app, fn_body, arg).await?;
    let text = match &result {
        Value::String(s) => s.clone(),
        _ => serde_json::to_string(&result).unwrap_or_else(|_| "null".to_string()),
    };
    Ok(json!({"content":[{"type":"text","text":text}]}).into())
}

// ---- JSON-RPC handler ----

fn handle_request(app: &AppHandle, req: &JsonRpcRequest) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0".into(), id: req.id.clone(),
            result: Some(json!({"protocolVersion":"2024-11-05","serverInfo":{"name":"webforge-mcp","version":"0.1.0"},"capabilities":{"tools":{}}})),
            error: None,
        },
        "initialized" => JsonRpcResponse { jsonrpc: "2.0".into(), id: req.id.clone(), result: Some(json!({})), error: None },
        "tools/list" => {
            let tools: Vec<Value> = tool_definitions().iter().map(|t| serde_json::to_value(t).unwrap()).collect();
            JsonRpcResponse { jsonrpc: "2.0".into(), id: req.id.clone(), result: Some(json!({"tools":tools})), error: None }
        }
        "tools/call" => {
            let params = req.params.as_ref().and_then(|p| p.as_object()).cloned().unwrap_or_default();
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let app = app.clone();
            let result_val = tauri::async_runtime::block_on(async { dispatch_tool(&app, tool_name, &args).await });
            match result_val {
                Ok(content) => JsonRpcResponse { jsonrpc: "2.0".into(), id: req.id.clone(), result: Some(json!({"content":content.get("content").cloned().unwrap_or(json!([]))})), error: None },
                Err(e) => JsonRpcResponse { jsonrpc: "2.0".into(), id: req.id.clone(), result: None, error: Some(JsonRpcError { code: -32603, message: e }) },
            }
        }
        _ => JsonRpcResponse { jsonrpc: "2.0".into(), id: req.id.clone(), result: None, error: Some(JsonRpcError { code: -32601, message: format!("Method not found: {}", req.method) }) },
    }
}

// ---- In-app TCP server (for Connect Agent tab) ----

pub fn start_mcp_server(app: AppHandle) -> Result<u16, String> {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let port_file = mcp_port_file();
    if let Some(p) = &port_file {
        if let Some(parent) = p.parent() { let _ = std::fs::create_dir_all(parent); }
        let _ = std::fs::write(p, port.to_string());
    }

    let app_clone = app.clone();
    std::thread::spawn(move || {
        eprintln!("[webforge-mcp] TCP server on port {}", port);
        for stream in listener.incoming() {
            let stream = match stream { Ok(s) => s, Err(_) => continue };
            let app = app_clone.clone();
            std::thread::spawn(move || { handle_tcp_connection(stream, app); });
        }
    });

    // Set up the Tauri event listener for mcp-eval-result
    let app_for_listen = app.clone();
    let app_for_handler = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Listener;
        let _ = app_for_listen.listen("mcp-eval-result", move |event| {
            if let Ok(payload) = serde_json::from_str::<Value>(event.payload()) {
                tauri::async_runtime::spawn(async move {
                    handle_eval_result(payload).await;
                });
            }
        });
        // keep app_for_handler alive
        std::mem::forget(app_for_handler);
    });

    Ok(port)
}

fn handle_tcp_connection(stream: std::net::TcpStream, app: AppHandle) {
    use std::io::{BufRead, Write};
    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;
    for line in reader.lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        if line.trim().is_empty() { continue; }
        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse { jsonrpc: "2.0".into(), id: Value::Null, result: None, error: Some(JsonRpcError { code: -32700, message: format!("Parse error: {}", e) }) };
                let _ = writeln!(writer, "{}", serde_json::to_string(&resp).unwrap());
                continue;
            }
        };
        let response = handle_request(&app, &req);
        let _ = writeln!(writer, "{}", serde_json::to_string(&response).unwrap());
        let _ = writer.flush();
    }
}

pub fn stop_mcp_server() -> Result<(), String> {
    if let Some(p) = &mcp_port_file() { let _ = std::fs::remove_file(p); }
    eprintln!("[webforge-mcp] stopped");
    Ok(())
}

fn mcp_port_file() -> Option<std::path::PathBuf> {
    dirs_next::home_dir().map(|h| {
        if cfg!(target_os = "macos") { h.join("Library/Application Support/com.webforge.studio/mcp-port") }
        else { h.join(".config/com.webforge.studio/mcp-port") }
    })
}

// ---- Standalone stdio proxy (`webforge-studio mcp` subcommand) ----

pub fn run_stdio_server_standalone() {
    use std::io::{BufRead, Write};
    use std::net::TcpStream;

    let mcp_port: u16 = mcp_port_file()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    if mcp_port == 0 {
        eprintln!("[webforge-mcp] Webforge Studio not running or MCP not started.");
        eprintln!("[webforge-mcp] Start the app → Connect Agent tab → Start MCP server.");
        std::process::exit(1);
    }

    let stream = match TcpStream::connect(("127.0.0.1", mcp_port)) {
        Ok(s) => s,
        Err(e) => { eprintln!("[webforge-mcp] Cannot connect (port {}): {}", mcp_port, e); std::process::exit(1); }
    };
    eprintln!("[webforge-mcp] Connected to Webforge Studio (port {})", mcp_port);

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        if line.trim().is_empty() { continue; }
        if writeln!(writer, "{}", line).is_err() { break; }
        let _ = writer.flush();
        let mut resp = String::new();
        if reader.read_line(&mut resp).is_err() { break; }
        let _ = write!(stdout, "{}", resp);
        let _ = stdout.flush();
    }
}