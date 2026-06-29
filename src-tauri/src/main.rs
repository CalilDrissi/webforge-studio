// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "mcp" {
        // MCP stdio subcommand — for external AI clients (Claude Desktop, Cursor)
        // Usage: webforge-studio mcp
        // The app must be running with at least one editor window open.
        // This subcommand connects to the running app via Tauri's app handle.
        // For now, we run the stdio server standalone (no Tauri app needed — it
        // drives the active editor window via the app's webview eval).
        eprintln!("[webforge-mcp] Starting MCP server on stdio...");
        eprintln!("[webforge-mcp] Make sure Webforge Studio is running with a workspace open.");
        // The stdio MCP server can't access the Tauri AppHandle from a separate
        // process. Instead, we use a simple HTTP bridge: the app exposes an
        // internal MCP endpoint, and this subcommand proxies stdio to it.
        // For now, run a minimal stdio loop that tells the user to configure
        // the app's Connect Agent tab instead.
        webforge_studio_lib::mcp::run_stdio_server_standalone();
        return;
    }
    webforge_studio_lib::run()
}