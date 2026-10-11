// Ubunye save-to-folder (added to every page by the web host, see index.js).
//
// The browser apps hand every export to the browser as a download (<a download href="blob:..">),
// so files always land in the Downloads folder. Where the browser can write to a folder the user
// picks (File System Access: Chrome, Edge, Opera, Brave on a computer), this catches those
// downloads and offers "Choose folder". The folder is remembered, so later exports are saved
// there without asking. Other browsers keep their normal download (their settings can ask
// where to save each file).
(() => {
  "use strict";
  if (typeof window.showDirectoryPicker !== "function" || !window.indexedDB) return;

  const nativeClick = HTMLAnchorElement.prototype.click;
  const DB = "ubunye-save";
  const KEY = "folder";
  const MODE = "ubunye-save-mode"; // "download" once the user chose plain downloads

  let folder = null; // FileSystemDirectoryHandle
  let queue = []; // { name, blob: Promise<Blob> }
  let box = null;
  const label = () => (folder && folder.name) || "your folder";
  let hideTimer = 0;

  // --- remembered folder (IndexedDB can store directory handles) ---
  function idb(mode, fn) {
    return new Promise((resolve, reject) => {
      const open = indexedDB.open(DB, 1);
      open.onupgradeneeded = () => open.result.createObjectStore("handles");
      open.onerror = () => reject(open.error);
      open.onsuccess = () => {
        const tx = open.result.transaction("handles", mode);
        const req = fn(tx.objectStore("handles"));
        tx.oncomplete = () => resolve(req && req.result);
        tx.onerror = () => reject(tx.error);
      };
    });
  }
  const loadFolder = () => idb("readonly", (s) => s.get(KEY)).catch(() => null);
  const storeFolder = (h) => idb("readwrite", (s) => s.put(h, KEY)).catch(() => {});
  const forgetFolder = () => idb("readwrite", (s) => s.delete(KEY)).catch(() => {});
  const ready = loadFolder().then((h) => { folder = h || null; });

  function getMode() {
    try { return localStorage.getItem(MODE); } catch { return null; }
  }
  function setMode(v) {
    try { v ? localStorage.setItem(MODE, v) : localStorage.removeItem(MODE); } catch { /* private mode */ }
  }

  async function permitted(h, ask) {
    const opts = { mode: "readwrite" };
    if ((await h.queryPermission(opts)) === "granted") return true;
    return ask && (await h.requestPermission(opts)) === "granted";
  }

  // --- writing ---
  async function freeName(dir, name) {
    const dot = name.lastIndexOf(".");
    const stem = dot > 0 ? name.slice(0, dot) : name;
    const ext = dot > 0 ? name.slice(dot) : "";
    for (let n = 0; n < 1000; n++) {
      const candidate = n === 0 ? name : `${stem} (${n})${ext}`;
      try {
        await dir.getFileHandle(candidate);
      } catch (e) {
        if (e && e.name === "NotFoundError") return candidate;
        throw e;
      }
    }
    return `${stem} (${Date.now()})${ext}`;
  }

  async function writeOne(item) {
    const blob = await item.blob;
    const name = await freeName(folder, item.name);
    const file = await folder.getFileHandle(name, { create: true });
    const out = await file.createWritable();
    try {
      await out.write(blob);
      await out.close();
    } catch (e) {
      await out.abort().catch(() => {});
      throw e;
    }
    return name;
  }

  let draining = false;
  async function drain() {
    if (draining) return;
    draining = true;
    try {
      let last = "";
      let count = 0;
      while (queue.length && folder) {
        const item = queue[0];
        try {
          last = await writeOne(item);
        } catch (e) {
          console.warn("[ubunye-save] could not save to the folder:", e);
          show({ error: `Could not save ${item.name} to "${label()}". Choose the folder again, or download it.` });
          return;
        }
        queue.shift();
        count++;
      }
      if (last) show({ done: last, count });
    } finally {
      draining = false;
    }
  }

  function downloadAll() {
    const items = queue;
    queue = [];
    for (const item of items) {
      item.blob.then((blob) => {
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = item.name;
        nativeClick.call(a);
        setTimeout(() => URL.revokeObjectURL(url), 60000);
      }, (e) => console.warn("[ubunye-save] lost an export:", e));
    }
  }

  async function chooseFolder() {
    let h;
    try {
      h = await window.showDirectoryPicker({ id: "ubunye-exports", mode: "readwrite", startIn: folder || "downloads" });
    } catch (e) {
      if (e && e.name === "AbortError") return; // cancelled: keep the files waiting
      show({ error: `This browser would not open a folder (${e && e.message}).` });
      return;
    }
    folder = h;
    setMode(null);
    storeFolder(h);
    drain();
  }

  async function useRemembered() {
    if (folder && (await permitted(folder, true))) drain();
    else chooseFolder();
  }

  // --- the little panel ---
  function el(tag, attrs, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (k === "on") e.addEventListener("click", v);
      else e.setAttribute(k, v);
    }
    for (const k of kids) e.append(k);
    return e;
  }

  function mount() {
    if (box) return box;
    const style = el("style", {}, `
#ubunye-save{position:fixed;right:16px;bottom:16px;z-index:2147483647;max-width:min(360px,calc(100vw - 32px));
 background:#14161a;color:#f2f2f2;border:1px solid #ffffff26;border-radius:12px;padding:14px 16px;
 font:14px/1.4 system-ui,-apple-system,"Segoe UI",sans-serif;box-shadow:0 8px 30px #0008}
#ubunye-save p{margin:0 0 10px}#ubunye-save small{color:#b8b8b8}
#ubunye-save .row{display:flex;flex-wrap:wrap;gap:8px}
#ubunye-save button{font:inherit;min-height:36px;padding:0 14px;border-radius:999px;cursor:pointer;
 border:1px solid #ffffff40;background:transparent;color:inherit}
#ubunye-save button.main{background:#f2f2f2;color:#14161a;border-color:#f2f2f2;font-weight:600}
#ubunye-save button.link{border:0;padding:0 4px;text-decoration:underline;color:#b8b8b8}
#ubunye-save .x{position:absolute;top:6px;right:8px;border:0;min-height:0;padding:4px 8px}`);
    box = el("div", { id: "ubunye-save", role: "status", "aria-live": "polite" });
    document.head.append(style);
    document.body.append(box);
    return box;
  }

  function hide() {
    if (box) box.style.display = "none";
  }

  function show(state) {
    const b = mount();
    clearTimeout(hideTimer);
    b.style.display = "";
    b.replaceChildren(el("button", { class: "x", "aria-label": "Close", on: hide }, "×"));
    const n = queue.length;
    const what = n === 1 ? queue[0].name : `${n} files`;
    if (state.done && !n) {
      b.append(
        el("p", {}, state.count === 1 ? `Saved ${state.done} to "${label()}".` : `Saved ${state.count} files to "${label()}".`),
        el("div", { class: "row" },
          el("button", { class: "link", on: chooseFolder }, "Change folder"),
          el("button", { class: "link", on: () => { folder = null; setMode("download"); forgetFolder(); hide(); } }, "Use Downloads instead")),
      );
      hideTimer = setTimeout(hide, 8000);
      return;
    }
    if (state.downloaded) {
      b.append(
        el("p", {}, `${state.downloaded} went to your Downloads folder.`),
        el("button", { class: "link", on: () => { setMode(null); hide(); } }, "Ask me for a folder next time"),
      );
      hideTimer = setTimeout(hide, 6000);
      return;
    }
    if (!n) return hide();
    const text = state.error ? state.error : `Your export is ready: ${what}. Where should it go?`;
    const remembered = folder && !state.error;
    b.append(
      el("p", {}, text, el("br"), el("small", {}, remembered ? "" : "We will remember the folder for next time.")),
      el("div", { class: "row" },
        remembered
          ? el("button", { class: "main", on: useRemembered }, `Save to "${label()}"`)
          : el("button", { class: "main", on: chooseFolder }, "Choose folder…"),
        remembered ? el("button", { on: chooseFolder }, "Other folder…") : "",
        el("button", { on: () => { const w = what; downloadAll(); show({ downloaded: w }); } }, "Download")),
    );
  }

  // --- catch the apps' downloads ---
  async function arrive(item) {
    queue.push(item);
    await ready;
    if (getMode() === "download") {
      downloadAll();
      return;
    }
    if (folder && !draining && (await permitted(folder, false).catch(() => false))) drain();
    else show({});
  }

  HTMLAnchorElement.prototype.click = function () {
    const href = this.href || "";
    if (this.hasAttribute("download") && href.startsWith("blob:")) {
      // Read the bytes now: the app revokes the blob URL a few seconds after this click.
      const blob = fetch(href).then((r) => r.blob());
      blob.catch(() => {});
      arrive({ name: this.getAttribute("download") || "export", blob });
      return;
    }
    return nativeClick.call(this);
  };
})();
