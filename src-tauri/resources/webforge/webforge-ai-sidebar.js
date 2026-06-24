// webforge-ai-sidebar.js
// Injected into editor.html by the embedded HTTP server.
// Creates an AI chat sidebar that lets the user give natural language instructions
// to modify the page. The AI sees the current page state and returns actions
// that are executed against the live Webforge.Builder API.
//
// Works in two modes:
// 1. Tauri mode: loads AI settings from the Tauri app's saved settings
// 2. Standalone mode: reads settings from localStorage (for use outside Tauri)

(function () {
  const SIDEBAR_ID = "webforge-ai-sidebar";
  let settings = null;
  let messages = []; // chat history
  let isProcessing = false;

  // ---- settings ----
  async function loadSettings() {
    // try Tauri first
    const invoke = window.__TAURI__?.core?.invoke;
    if (invoke) {
      try {
        const s = await invoke("load_ai_settings");
        if (s) {
          settings = { provider: s.provider, baseUrl: s.baseUrl, apiKey: s.apiKey, model: s.model };
          return;
        }
      } catch (e) { /* fall through to localStorage */ }
    }
    // standalone: localStorage
    try {
      const raw = localStorage.getItem("webforge-ai-settings");
      if (raw) settings = JSON.parse(raw);
    } catch (e) {}
  }

  function saveSettingsLocal(s) {
    settings = s;
    try { localStorage.setItem("webforge-ai-settings", JSON.stringify(s)); } catch (e) {}
  }

  // ---- page context gathering ----
  function getPageContext() {
    const ctx = {
      selectedElement: null,
      pageStructure: [],
      availableSections: [],
      availableComponents: [],
      currentHtml: null,
    };

    try {
      // selected element info
      const el = Webforge.Builder.selectedEl;
      if (el) {
        ctx.selectedElement = {
          tag: el.tagName,
          id: el.id || null,
          className: el.className || null,
          textPreview: (el.textContent || "").slice(0, 100),
          htmlPreview: el.outerHTML.slice(0, 300),
        };
      }

      // page structure (top-level elements in the iframe body)
      const doc = Webforge.Builder.frameDoc;
      if (doc && doc.body) {
        ctx.pageStructure = Array.from(doc.body.children).map((c) => ({
          tag: c.tagName,
          id: c.id || null,
          class: (c.className || "").slice(0, 80),
          children: c.children.length,
        }));
      }

      // available sections (limited list)
      const sectionGroups = Webforge.SectionsGroup || {};
      for (const group in sectionGroups) {
        for (const type of sectionGroups[group].slice(0, 20)) {
          const s = Webforge.Sections.get(type);
          ctx.availableSections.push({ type, name: s ? s.name : type });
        }
      }

      // available components (limited)
      const compGroups = Webforge.ComponentsGroup || {};
      for (const group in compGroups) {
        for (const type of compGroups[group].slice(0, 15)) {
          const c = Webforge.Components.get(type);
          ctx.availableComponents.push({ type, name: c ? c.name : type });
        }
      }

      // current HTML (truncated for context window)
      const fullHtml = Webforge.Builder.getHtml(false, true);
      ctx.currentHtml = fullHtml.length > 4000 ? fullHtml.slice(0, 4000) + "\n<!-- truncated -->" : fullHtml;
    } catch (e) {
      ctx.error = e.message;
    }

    return ctx;
  }

  // ---- AI call ----
  async function callAI(userMessage) {
    if (!settings) throw new Error("AI not configured. Click the gear icon to set your API key.");

    const ctx = getPageContext();

    const systemPrompt = `You are an AI assistant inside the Webforge visual page builder. You can see the current page state and modify it by returning actions.

Available Webforge.Builder API:
- Webforge.Builder.getHtml(keepHelpers) - get current page HTML
- Webforge.Builder.setHtml(html) - replace entire page
- Webforge.Builder.selectNode(el) - select an element
- Webforge.Builder.selectedEl - currently selected element (or null)
- Webforge.Builder.frameDoc - the iframe document being edited
- Webforge.Builder.reloadComponent() - refresh properties panel
- Webforge.Undo.addMutation(m) / Webforge.Undo.undo() / Webforge.Undo.redo()
- Webforge.Sections.get(type) / Webforge.SectionsGroup - section library
- Webforge.Components.get(type) / Webforge.ComponentsGroup - component library
- Webforge.Blocks.get(type) / Webforge.BlocksGroup - block library

You MUST respond with a JSON object:
{
  "thinking": "Brief explanation of what you're doing",
  "actions": [
    { "tool": "select_element", "args": { "selector": "header" } },
    { "tool": "set_property", "args": { "property_type": "style", "name": "background-color", "value": "#1a1a2e" } },
    { "tool": "add_component", "args": { "component_type": "bootstrap5/hero-1", "kind": "section" } },
    { "tool": "delete_element", "args": {} },
    { "tool": "set_html", "args": { "html": "<!doctype html>..." } },
    { "tool": "set_property", "args": { "property_type": "text", "name": "", "value": "New heading text" } },
    { "tool": "set_property", "args": { "property_type": "attribute", "name": "class", "value": "btn btn-primary btn-lg" } },
    { "tool": "set_property", "args": { "property_type": "html", "name": "", "value": "<h2>New content</h2><p>Paragraph</p>" } }
  ],
  "message": "User-facing message explaining what you did"
}

Rules:
1. Return ONLY valid JSON (no markdown, no code blocks)
2. Use select_element before set_property or delete_element
3. For add_component, use types from the available sections/components lists below
4. Keep actions focused on what the user asked
5. The "message" field is shown to the user in the chat
6. Prefer multiple small actions over one big set_html when possible

Current page context:
${JSON.stringify(ctx, null, 2)}`;

    const userPayload = {
      model: settings.model,
      messages: [
        { role: "system", content: systemPrompt },
        ...messages.slice(-8), // keep last 8 messages for context
        { role: "user", content: userMessage },
      ],
      temperature: 0.3,
      stream: false,
    };

    const url = settings.baseUrl.replace(/\/+$/, "") + "/chat/completions";
    const res = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${settings.apiKey}`,
      },
      body: JSON.stringify(userPayload),
    });

    if (!res.ok) {
      const text = await res.text();
      throw new Error(`AI API error (${res.status}): ${text.slice(0, 200)}`);
    }

    const data = await res.json();
    const content = data.choices?.[0]?.message?.content;
    if (!content) throw new Error("AI returned no content");

    return content;
  }

  // ---- action execution ----
  function executeAction(action) {
    const { tool, args } = action;
    const doc = Webforge.Builder.frameDoc;

    switch (tool) {
      case "select_element": {
        const el = doc.querySelector(args.selector);
        if (!el) return { error: `Element not found: ${args.selector}` };
        Webforge.Builder.selectNode(el);
        return { success: true, message: `Selected ${el.tagName}` };
      }

      case "set_property": {
        const el = Webforge.Builder.selectedEl;
        if (!el) return { error: "No element selected" };
        if (args.property_type === "attribute") {
          const old = el.getAttribute(args.name);
          el.setAttribute(args.name, args.value);
          Webforge.Undo.addMutation({ type: "attributes", target: el, attributeName: args.name, oldValue: old, newValue: args.value });
        } else if (args.property_type === "style") {
          const old = el.style[args.name];
          el.style[args.name] = args.value;
          Webforge.Undo.addMutation({ type: "style", target: el, attributeName: args.name, oldValue: old, newValue: args.value });
        } else if (args.property_type === "text") {
          const old = el.textContent;
          el.textContent = args.value;
          Webforge.Undo.addMutation({ type: "characterData", target: el, oldValue: old, newValue: args.value });
        } else if (args.property_type === "html") {
          const old = el.innerHTML;
          el.innerHTML = args.value;
          Webforge.Undo.addMutation({ type: "childList", target: el, addedNodes: Array.from(el.children) });
        }
        Webforge.Builder.reloadComponent();
        return { success: true, message: `Set ${args.property_type} ${args.name || ""} = ${args.value}` };
      }

      case "add_component": {
        let data;
        if (args.kind === "section") data = Webforge.Sections.get(args.component_type);
        else if (args.kind === "block") data = Webforge.Blocks.get(args.component_type);
        else data = Webforge.Components.get(args.component_type);
        if (!data) return { error: `Component not found: ${args.component_type}` };
        const html = data.html || data.dragHtml || "";
        if (!html) return { error: "Component has no html" };
        const wrapper = doc.createElement("div");
        wrapper.innerHTML = html;
        const node = wrapper.firstElementChild;
        if (node) {
          const target = Webforge.Builder.selectedEl || doc.body;
          if (target && target !== doc.body) {
            target.parentNode.insertBefore(node, target.nextSibling);
          } else {
            doc.body.appendChild(node);
          }
          Webforge.Undo.addMutation({ type: "childList", target: node.parentNode, addedNodes: [node] });
          Webforge.Builder.selectNode(node);
          if (Webforge.TreeList) Webforge.TreeList.loadComponents();
        }
        return { success: true, message: `Added ${args.component_type}` };
      }

      case "delete_element": {
        const el = Webforge.Builder.selectedEl;
        if (!el) return { error: "No element selected" };
        const parent = el.parentNode;
        if (!parent) return { error: "Cannot delete root" };
        Webforge.Undo.addMutation({ type: "childList", target: parent, removedNodes: [el], nextSibling: el.nextSibling });
        el.remove();
        Webforge.Builder.selectNode(null);
        if (Webforge.TreeList) Webforge.TreeList.loadComponents();
        return { success: true, message: `Deleted ${el.tagName}` };
      }

      case "set_html": {
        Webforge.Builder.setHtml(args.html);
        Webforge.Undo.reset();
        return { success: true, message: "Page HTML replaced" };
      }

      case "undo": {
        Webforge.Undo.undo();
        return { success: true, message: "Undone" };
      }

      case "redo": {
        Webforge.Undo.redo();
        return { success: true, message: "Redone" };
      }

      default:
        return { error: `Unknown action: ${tool}` };
    }
  }

  function parseAIResponse(content) {
    // extract JSON from the response
    let jsonStr = null;
    try {
      JSON.parse(content.trim());
      jsonStr = content.trim();
    } catch (e) {
      // try ```json block
      const m = content.match(/```json\s*([\s\S]*?)```/);
      if (m) {
        try { JSON.parse(m[1].trim()); jsonStr = m[1].trim(); } catch (e2) {}
      }
      if (!jsonStr) {
        // try ``` block
        const m2 = content.match(/```\s*([\s\S]*?)```/);
        if (m2) { try { JSON.parse(m2[1].trim()); jsonStr = m2[1].trim(); } catch (e3) {} }
      }
      if (!jsonStr) {
        const s = content.indexOf("{"), eEnd = content.lastIndexOf("}");
        if (s >= 0 && eEnd > s) { try { JSON.parse(content.slice(s, eEnd + 1)); jsonStr = content.slice(s, eEnd + 1); } catch (e4) {} }
      }
    }
    if (!jsonStr) return null;
    return JSON.parse(jsonStr);
  }

  // ---- UI ----
  function createSidebar() {
    if (document.getElementById(SIDEBAR_ID)) return;

    const sidebar = document.createElement("div");
    sidebar.id = SIDEBAR_ID;
    sidebar.innerHTML = `
      <div class="wf-ai-header">
        <span class="wf-ai-title">AI Assistant</span>
        <div class="wf-ai-header-actions">
          <button class="wf-ai-icon-btn" id="wf-ai-settings-btn" title="AI Settings">&#9881;</button>
          <button class="wf-ai-icon-btn" id="wf-ai-toggle-btn" title="Collapse">&#9776;</button>
        </div>
      </div>
      <div class="wf-ai-settings-panel" id="wf-ai-settings-panel" style="display:none">
        <label>Provider
          <select id="wf-ai-provider">
            <option value="openai">OpenAI</option>
            <option value="ollama-cloud">Ollama Cloud</option>
            <option value="ollama-local">Ollama Local</option>
            <option value="groq">Groq</option>
            <option value="together">Together</option>
            <option value="openrouter">OpenRouter</option>
            <option value="custom">Custom</option>
          </select>
        </label>
        <label>Base URL <input type="text" id="wf-ai-base-url" placeholder="https://api.openai.com/v1" /></label>
        <label>API Key <input type="password" id="wf-ai-api-key" placeholder="sk-..." /></label>
        <label>Model <input type="text" id="wf-ai-model" placeholder="gpt-4o" /></label>
        <button class="wf-ai-save-btn" id="wf-ai-save-settings">Save</button>
      </div>
      <div class="wf-ai-messages" id="wf-ai-messages"></div>
      <div class="wf-ai-input-row">
        <textarea id="wf-ai-input" placeholder="Ask AI to modify the page..." rows="2"></textarea>
        <button id="wf-ai-send" class="wf-ai-send-btn">&#8593;</button>
      </div>
    `;

    document.body.appendChild(sidebar);
    injectStyles();

    // wire up events
    const messagesEl = document.getElementById("wf-ai-messages");
    const inputEl = document.getElementById("wf-ai-input");
    const sendBtn = document.getElementById("wf-ai-send");
    const toggleBtn = document.getElementById("wf-ai-toggle-btn");
    const settingsBtn = document.getElementById("wf-ai-settings-btn");
    const settingsPanel = document.getElementById("wf-ai-settings-panel");
    const providerSelect = document.getElementById("wf-ai-provider");
    const baseUrlInput = document.getElementById("wf-ai-base-url");
    const apiKeyInput = document.getElementById("wf-ai-api-key");
    const modelInput = document.getElementById("wf-ai-model");
    const saveSettingsBtn = document.getElementById("wf-ai-save-settings");

    const presets = {
      "openai": { url: "https://api.openai.com/v1", model: "gpt-4o" },
      "ollama-cloud": { url: "https://api.ovhcloud.com/ollama/v1", model: "llama3.1" },
      "ollama-local": { url: "http://localhost:11434/v1", model: "llama3.1" },
      "groq": { url: "https://api.groq.com/openai/v1", model: "llama-3.3-70b-versatile" },
      "together": { url: "https://api.together.xyz/v1", model: "meta-llama/Llama-3.3-70B-Instruct-Turbo" },
      "openrouter": { url: "https://openrouter.ai/api/v1", model: "anthropic/claude-3.5-sonnet" },
      "custom": { url: "", model: "" },
    };

    // load saved settings into the form
    if (settings) {
      providerSelect.value = settings.provider || "openai";
      baseUrlInput.value = settings.baseUrl || "";
      apiKeyInput.value = settings.apiKey || "";
      modelInput.value = settings.model || "";
    }

    providerSelect.addEventListener("change", () => {
      const p = presets[providerSelect.value];
      if (p) {
        if (p.url) baseUrlInput.value = p.url;
        if (p.model) modelInput.value = p.model;
      }
    });

    settingsBtn.addEventListener("click", () => {
      settingsPanel.style.display = settingsPanel.style.display === "none" ? "block" : "none";
    });

    saveSettingsBtn.addEventListener("click", () => {
      const s = {
        provider: providerSelect.value,
        baseUrl: baseUrlInput.value.trim(),
        apiKey: apiKeyInput.value.trim(),
        model: modelInput.value.trim(),
      };
      saveSettingsLocal(s);
      settingsPanel.style.display = "none";
      addMessage("system", "Settings saved. You can now chat with the AI.");
    });

    toggleBtn.addEventListener("click", () => {
      sidebar.classList.toggle("wf-ai-collapsed");
      toggleBtn.innerHTML = sidebar.classList.contains("wf-ai-collapsed") ? "&#9776;" : "&#9776;";
    });

    async function handleSend() {
      const text = inputEl.value.trim();
      if (!text || isProcessing) return;
      if (!settings || !settings.apiKey) {
        settingsPanel.style.display = "block";
        addMessage("error", "Please configure your AI API key first.");
        return;
      }

      inputEl.value = "";
      addMessage("user", text);
      messages.push({ role: "user", content: text });

      isProcessing = true;
      sendBtn.disabled = true;
      const thinkingEl = addMessage("thinking", "Thinking...");

      try {
        const response = await callAI(text);
        const parsed = parseAIResponse(response);

        if (parsed && parsed.actions) {
          // show AI's thinking
          if (parsed.thinking) addMessage("ai-thinking", parsed.thinking);

          // execute each action
          const results = [];
          for (const action of parsed.actions) {
            const result = executeAction(action);
            results.push({ tool: action.tool, ...result });
          }

          // show user-facing message
          const msgText = parsed.message || `Executed ${parsed.actions.length} actions`;
          addMessage("assistant", msgText);

          // show action results
          const resultsText = results
            .map((r) => (r.success ? `\u2705 ${r.message}` : `\u274c ${r.error}`))
            .join("\n");
          addMessage("actions", resultsText);

          messages.push({ role: "assistant", content: msgText });
        } else {
          // no JSON, just show raw response
          addMessage("assistant", response.slice(0, 500));
          messages.push({ role: "assistant", content: response.slice(0, 500) });
        }

        thinkingEl.remove();
      } catch (e) {
        thinkingEl.remove();
        addMessage("error", e.message);
      } finally {
        isProcessing = false;
        sendBtn.disabled = false;
      }
    }

    sendBtn.addEventListener("click", handleSend);
    inputEl.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    });
  }

  function addMessage(role, text) {
    const messagesEl = document.getElementById("wf-ai-messages");
    if (!messagesEl) return null;
    const msg = document.createElement("div");
    msg.className = `wf-ai-msg wf-ai-msg-${role}`;
    if (role === "thinking") {
      msg.innerHTML = '<span class="wf-ai-spinner"></span> ' + escapeHtml(text);
    } else if (role === "actions") {
      msg.innerHTML = '<pre class="wf-ai-actions">' + escapeHtml(text) + "</pre>";
    } else {
      msg.textContent = text;
    }
    messagesEl.appendChild(msg);
    messagesEl.scrollTop = messagesEl.scrollHeight;
    return msg;
  }

  function escapeHtml(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  function injectStyles() {
    const style = document.createElement("style");
    style.textContent = `
      #${SIDEBAR_ID} {
        position: fixed; top: 0; right: 0; bottom: 0; width: 360px;
        background: #0f1115; border-left: 1px solid #2a2f3a;
        display: flex; flex-direction: column; z-index: 99998;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
        font-size: 13px; color: #e6e9ef; transition: transform 0.2s ease;
      }
      #${SIDEBAR_ID}.wf-ai-collapsed { transform: translateX(340px); }
      .wf-ai-header {
        display: flex; justify-content: space-between; align-items: center;
        padding: 12px 16px; border-bottom: 1px solid #2a2f3a; background: #171a21;
      }
      .wf-ai-title { font-weight: 600; font-size: 14px; }
      .wf-ai-header-actions { display: flex; gap: 4px; }
      .wf-ai-icon-btn {
        background: none; border: none; color: #8a93a4; cursor: pointer;
        padding: 4px 8px; border-radius: 4px; font-size: 16px;
      }
      .wf-ai-icon-btn:hover { background: #1f2430; color: #e6e9ef; }
      .wf-ai-settings-panel {
        padding: 12px 16px; border-bottom: 1px solid #2a2f3a; background: #171a21;
        display: flex; flex-direction: column; gap: 8px;
      }
      .wf-ai-settings-panel label {
        display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: #8a93a4;
      }
      .wf-ai-settings-panel input, .wf-ai-settings-panel select {
        background: #0f1115; border: 1px solid #2a2f3a; color: #e6e9ef;
        padding: 6px 10px; border-radius: 4px; font-size: 12px; outline: none;
      }
      .wf-ai-settings-panel input:focus, .wf-ai-settings-panel select:focus { border-color: #4c8dff; }
      .wf-ai-save-btn {
        background: #4c8dff; color: white; border: none; padding: 8px 12px;
        border-radius: 4px; cursor: pointer; font-size: 12px; font-weight: 600;
      }
      .wf-ai-save-btn:hover { background: #6ba0ff; }
      .wf-ai-messages {
        flex: 1; overflow-y: auto; padding: 12px 16px; display: flex;
        flex-direction: column; gap: 8px;
      }
      .wf-ai-msg {
        padding: 8px 12px; border-radius: 8px; max-width: 90%; word-wrap: break-word;
        line-height: 1.5; white-space: pre-wrap;
      }
      .wf-ai-msg-user { background: #4c8dff; color: white; align-self: flex-end; }
      .wf-ai-msg-assistant { background: #1f2430; align-self: flex-start; }
      .wf-ai-msg-thinking { background: #1f2430; color: #8a93a4; font-style: italic; align-self: flex-start; }
      .wf-ai-msg-ai-thinking { background: #171a21; border-left: 3px solid #4c8dff; color: #8a93a4; font-size: 12px; align-self: flex-start; }
      .wf-ai-msg-error { background: rgba(224,86,86,0.15); color: #e05656; border: 1px solid rgba(224,86,86,0.3); align-self: flex-start; }
      .wf-ai-msg-actions { background: #0a0c12; border: 1px solid #2a2f3a; align-self: flex-start; padding: 0; }
      .wf-ai-actions { margin: 0; padding: 8px 12px; font-size: 11px; color: #b8c0d0; font-family: ui-monospace, monospace; }
      .wf-ai-spinner {
        display: inline-block; width: 12px; height: 12px;
        border: 2px solid #4c8dff; border-top: 2px solid transparent;
        border-radius: 50%; animation: wf-ai-spin 0.8s linear infinite;
      }
      @keyframes wf-ai-spin { to { transform: rotate(360deg); } }
      .wf-ai-input-row {
        display: flex; gap: 8px; padding: 12px 16px; border-top: 1px solid #2a2f3a; background: #171a21;
      }
      .wf-ai-input-row textarea {
        flex: 1; background: #0f1115; border: 1px solid #2a2f3a; color: #e6e9ef;
        padding: 8px 12px; border-radius: 4px; font-size: 13px; resize: none; outline: none;
        font-family: inherit;
      }
      .wf-ai-input-row textarea:focus { border-color: #4c8dff; }
      .wf-ai-send-btn {
        background: #4c8dff; color: white; border: none; padding: 0 16px;
        border-radius: 4px; cursor: pointer; font-size: 18px; font-weight: 700;
      }
      .wf-ai-send-btn:hover { background: #6ba0ff; }
      .wf-ai-send-btn:disabled { opacity: 0.4; cursor: not-allowed; }

      /* shift the editor layout to make room for the sidebar */
      body.has-ai-sidebar { margin-right: 360px; }
    `;
    document.head.appendChild(style);
  }

  // ---- init ----
  function waitForReady(tries = 0) {
    if (window.Webforge && window.Webforge.Builder && window.Webforge.Builder.iframe) {
      createSidebar();
      document.body.classList.add("has-ai-sidebar");
      loadSettings().then(() => {
        if (!settings) {
          addMessage("system", "Welcome to the Webforge AI Assistant! Click the gear icon to configure your AI provider.");
        } else {
          addMessage("system", `AI Assistant ready (${settings.provider}/${settings.model}). Ask me to modify the page!`);
        }
      });
    } else if (tries < 100) {
      setTimeout(() => waitForReady(tries + 1), 100);
    }
  }

  waitForReady();
})();