// webforge-bridge.js
// Injected into editor.html by the embedded HTTP server.
// Routes save/upload/scan through the Rust-native /api/* endpoints on the embedded
// HTTP server (Phase A — PHP replacement). The site id is read from query params
// set by open_site, and every /api request carries ?site=<id> so the server can
// resolve the on-disk folder.
//
// Conflict detection (mtime check) and upload progress events still use Tauri
// commands because they need filesystem access / event emission the HTTP layer
// doesn't provide.

(function () {
  const tauriInvoke = window.__TAURI__?.core?.invoke || window.__TAURI_INVOKE__ || window.invoke;
  const listen = window.__TAURI__?.event?.listen || window.__TAURI__?.event?.default?.listen;
  const stripPath = (s) => s.split("?")[0].split("#")[0];

  // read site context from query string: ?site=<id>&kind=<local|remote>&startUrl=<...>&folder=<...>&server=<...>
  const params = new URLSearchParams(window.location.search);
  const siteId = params.get("site");
  const kind = params.get("kind") || "local";
  const startUrl = params.get("startUrl");
  const folderPath = params.get("folder") || null;
  const serverUrl = params.get("server") || null;

  console.log("[webforge-bridge] active", { siteId, kind, startUrl, folderPath, serverUrl });

  // For local sites, every /api call must carry ?site=<id>. For remote sites we still
  // keep the legacy Tauri-command path (the embedded server doesn't proxy remote saves).
  const apiBase = `/api/save?site=${encodeURIComponent(siteId || "")}`;
  const uploadApiBase = `/api/upload?site=${encodeURIComponent(siteId || "")}`;
  const scanApiBase = `/api/scan?site=${encodeURIComponent(siteId || "")}`;

  // ---- upload progress bar UI ----
  const progressEl = document.createElement("div");
  progressEl.id = "wf-upload-progress";
  progressEl.style.cssText = [
    "position:fixed",
    "bottom:0",
    "left:0",
    "right:0",
    "height:4px",
    "background:#1f2430",
    "z-index:99999",
    "display:none",
    "transition:width 0.2s ease",
  ].join(";");
  const progressFill = document.createElement("div");
  progressFill.style.cssText = [
    "height:100%",
    "width:0%",
    "background:#4c8dff",
    "transition:width 0.2s ease",
  ].join(";");
  progressEl.appendChild(progressFill);
  document.addEventListener("DOMContentLoaded", () => document.body.appendChild(progressEl));

  function showProgress(percent) {
    if (percent >= 100 || percent <= 0) {
      if (percent >= 100) {
        progressFill.style.width = "100%";
        setTimeout(() => { progressEl.style.display = "none"; progressFill.style.width = "0%"; }, 400);
      }
      return;
    }
    progressEl.style.display = "block";
    progressFill.style.width = percent + "%";
  }

  if (listen) {
    listen("upload-progress", (event) => {
      const p = event.payload;
      if (p && typeof p.percent === "number") {
        showProgress(p.percent);
        console.log(`[webforge-bridge] upload ${p.filename}: ${p.percent}% (${p.uploaded}/${p.total})`);
      }
    }).catch((e) => console.warn("[webforge-bridge] progress listener failed", e));
  }

  // ---- track the mtime of the currently-loaded local file for conflict detection ----
  let loadedFileMtime = null;
  async function refreshLoadedMtime() {
    if (kind !== "local" || !folderPath || !tauriInvoke) return;
    try {
      const src = stripPath(Webforge.Builder.iframe?.src || "");
      const match = src.match(/\/site\/\d+\/(.+)$/);
      const file = match ? match[1] : startUrl || "index.html";
      loadedFileMtime = await tauriInvoke("get_file_mtime", { folderPath, file });
    } catch (e) {
      console.warn("[webforge-bridge] mtime check failed", e);
      loadedFileMtime = null;
    }
  }

  function waitForWebforge(cb, tries = 0) {
    if (window.Webforge && window.Webforge.Builder && typeof window.Webforge.Builder.saveAjax === "function") {
      cb();
    } else if (tries < 100) {
      setTimeout(() => waitForWebforge(cb, tries + 1), 50);
    } else {
      console.warn("[webforge-bridge] Webforge did not initialize in time");
    }
  }

  // ---- legacy Tauri-command save/upload fallback (used for remote sites) ----
  async function tauriSave(args) {
    if (!tauriInvoke) throw new Error("Tauri invoke unavailable");
    return tauriInvoke("save_site", { args });
  }
  async function tauriUpload(args) {
    if (!tauriInvoke) throw new Error("Tauri invoke unavailable");
    return tauriInvoke("upload_site", { args });
  }

  waitForWebforge(() => {
    // ---- override saveAjax ----
    const originalSaveAjax = Webforge.Builder.saveAjax;
    Webforge.Builder.saveAjax = async function (data, saveUrl, callback, error) {
      try {
        let file = data && data.file;
        let html = data && data.html;
        if (!html) {
          const doc = Webforge.Builder.frameDoc;
          if (doc) {
            html = "<!doctype html>\n" + doc.documentElement.outerHTML;
          }
        }
        if (!file) {
          const src = Webforge.Builder.iframe?.src || "";
          const localMatch = stripPath(src).match(/\/site\/\d+\/(.+)$/);
          file = localMatch ? localMatch[1] : (startUrl || "index.html");
        }

        // For local sites: use the /api/save HTTP route (Rust-native, no Tauri command).
        // For remote sites: fall back to the Tauri save_site command which proxies.
        let result;
        if (kind === "local" && siteId) {
          const form = new URLSearchParams();
          form.set("file", file);
          form.set("html", html);
          if (data && data.startTemplateUrl) form.set("startTemplateUrl", data.startTemplateUrl);
          const res = await fetch(apiBase, {
            method: "POST",
            headers: { "Content-Type": "application/x-www-form-urlencoded" },
            body: form.toString(),
          });
          const text = await res.text();
          result = { success: res.ok, message: text, savedFile: file };
          if (!res.ok) throw new Error(text);
        } else {
          result = await tauriSave({
            kind,
            file,
            html,
            folderPath,
            serverUrl,
            expectedMtime: kind === "local" ? loadedFileMtime : null,
          });
        }

        // conflict detection only applies to the Tauri-command path (remote proxy)
        if (result.conflict) {
          const ok = confirm(
            "This file was modified on disk since you loaded it (saved at " +
            new Date(result.conflict.actualMtime * 1000).toLocaleString() +
            ").\n\nOverwrite the disk version anyway? (Your edits will replace the newer disk content.)"
          );
          if (ok) {
            const retry = await tauriSave({
              kind, file, html, folderPath, serverUrl, expectedMtime: null,
            });
            if (retry.success) loadedFileMtime = await tauriInvoke("get_file_mtime", { folderPath, file }).catch(() => null);
            if (typeof callback === "function") callback(retry);
            window.dispatchEvent(new CustomEvent("webforge.Builder.saveAjax", { detail: retry }));
            return retry;
          } else {
            if (typeof error === "function") error("Save cancelled due to file conflict");
            return result;
          }
        }

        if (result.success && kind === "local" && tauriInvoke) {
          loadedFileMtime = await tauriInvoke("get_file_mtime", { folderPath, file }).catch(() => null);
        }
        if (typeof callback === "function") callback(result);
        window.dispatchEvent(new CustomEvent("webforge.Builder.saveAjax", { detail: result }));
        return result;
      } catch (e) {
        console.error("[webforge-bridge] save failed", e);
        if (typeof error === "function") error(e);
        return originalSaveAjax.call(this, data, saveUrl, callback, error);
      }
    };

    // ---- override upload via fetch interception ----
    // For local sites: re-post to /api/upload (multipart) — the Rust handler validates
    // extensions and writes to <site-root>/<mediaPath>/<filename>.
    // For remote sites: keep the legacy Tauri upload_site command path.
    const originalFetch = window.fetch;
    window.fetch = async function (input, init) {
      const url = typeof input === "string" ? input : input?.url;
      if (url && /upload\.php(\?|$)/.test(url) && init && init.method && init.method.toUpperCase() === "POST") {
        try {
          const body = init.body;
          if (kind === "local" && siteId && body instanceof FormData) {
            // pass the FormData straight through to the Rust /api/upload endpoint
            showProgress(1);
            const res = await originalFetch(uploadApiBase, {
              method: "POST",
              body, // let the browser set the multipart boundary
            });
            showProgress(100);
            // webforge's upload handler expects a plain-text response containing the src URL
            const text = await res.text();
            return new Response(text, { status: res.status, headers: { "Content-Type": "text/plain" } });
          }

          // remote path: legacy Tauri command
          let fileBytes = null;
          let filename = "upload.bin";
          let mime = "application/octet-stream";
          if (body instanceof FormData) {
            for (const [, value] of body.entries()) {
              if (value instanceof File || value instanceof Blob) {
                fileBytes = new Uint8Array(await value.arrayBuffer());
                filename = value.name || filename;
                mime = value.type || mime;
                break;
              }
            }
          }
          if (!fileBytes) {
            return originalFetch.apply(this, arguments);
          }
          showProgress(1);
          const result = await tauriUpload({
            kind, filename, mime, bytes: Array.from(fileBytes), folderPath, serverUrl,
          });
          showProgress(100);
          return new Response(JSON.stringify(result), {
            status: 200,
            headers: { "Content-Type": "application/json" },
          });
        } catch (e) {
          console.error("[webforge-bridge] upload failed, falling back", e);
          showProgress(0);
          return originalFetch.apply(this, arguments);
        }
      }

      // ---- scan.php → /api/scan ----
      if (url && /scan\.php(\?|$)/.test(url) && kind === "local" && siteId) {
        // rewrite to /api/scan?site=<id>&mediaPath=...
        const u = new URL(url, window.location.origin);
        const mediaPath = u.searchParams.get("mediaPath") || "media";
        const newUrl = `${scanApiBase}&mediaPath=${encodeURIComponent(mediaPath)}`;
        return originalFetch(newUrl, init);
      }

      return originalFetch.apply(this, arguments);
    };

    // ---- load the iframe ----
    if (kind === "local" && startUrl) {
      const tryLoad = (tries = 0) => {
        if (Webforge.Builder && Webforge.Builder.iframe) {
          Webforge.Builder.iframe.src = "/site/" + encodeURIComponent(siteId) + "/" + startUrl;
          Webforge.Builder.iframe.addEventListener("load", () => {
            setTimeout(refreshLoadedMtime, 200);
          }, { once: true });
        } else if (tries < 50) {
          setTimeout(() => tryLoad(tries + 1), 50);
        }
      };
      tryLoad();
    } else if (kind === "remote" && startUrl) {
      const tryLoad = (tries = 0) => {
        if (Webforge.Builder && Webforge.Builder.iframe) {
          Webforge.Builder.iframe.src = "/proxy/" + encodeURIComponent(startUrl);
        } else if (tries < 50) {
          setTimeout(() => tryLoad(tries + 1), 50);
        }
      };
      tryLoad();
    }

    console.log("[webforge-bridge] saveAjax + upload + scan routed to /api/* (Phase A)");
  });
})();