// Serves the browser builds of Ubunye Light, Photo and Film from the R2 bucket
// ubunye-creative-tools (keys web/<app>/<path>). The builds are static sites, but their
// .wasm files are bigger than the 25 MiB per-file limit of Pages and Workers assets.
//
// Every page also gets /__ubunye/save.js (save.client.js): exports can go to a folder the user
// picks instead of always landing in Downloads. It lives here, not in the apps, so the forks stay
// close to upstream and new web builds get it without a rebuild.
import SAVE_JS from "./save.client.js";

const SAVE_PATH = "/__ubunye/save.js";
const SAVE_ETAG = `"save-${hash(SAVE_JS)}"`;

function hash(text) {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619);
  return (h >>> 0).toString(36);
}
const HOSTS = {
  "light.tools.thabangvision.com": "light",
  "photo.tools.thabangvision.com": "photo",
  "film.tools.thabangvision.com": "film",
};

const TYPES = {
  html: "text/html; charset=utf-8",
  js: "text/javascript; charset=utf-8",
  mjs: "text/javascript; charset=utf-8",
  wasm: "application/wasm",
  css: "text/css; charset=utf-8",
  json: "application/json",
  png: "image/png",
  svg: "image/svg+xml",
  ico: "image/x-icon",
  otf: "font/otf",
  ttf: "font/ttf",
  woff2: "font/woff2",
  txt: "text/plain; charset=utf-8",
  md: "text/plain; charset=utf-8",
};

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const app = HOSTS[url.hostname];
    if (!app) return new Response("Not found", { status: 404 });
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response("Method not allowed", { status: 405, headers: { Allow: "GET, HEAD" } });
    }

    if (url.pathname === SAVE_PATH) {
      const headers = {
        "content-type": "text/javascript; charset=utf-8",
        "cache-control": "no-cache",
        etag: SAVE_ETAG,
        "x-content-type-options": "nosniff",
      };
      if (request.headers.get("if-none-match") === SAVE_ETAG) return new Response(null, { status: 304, headers });
      return new Response(request.method === "HEAD" ? null : SAVE_JS, { headers });
    }

    let path = decodeURIComponent(url.pathname);
    if (path.endsWith("/")) path += "index.html";
    if (path.split("/").includes("..")) return new Response("Not found", { status: 404 });

    const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
    // Pages are rewritten below, so they are always read in full.
    const obj = await env.BUCKET.get(`web/${app}${path}`, ext === "html" ? {} : { onlyIf: request.headers });
    if (!obj) return new Response("Not found", { status: 404 });

    const headers = new Headers();
    obj.writeHttpMetadata(headers);
    headers.set("etag", obj.httpEtag);
    if (TYPES[ext]) headers.set("content-type", TYPES[ext]);
    // File names stay the same between versions, so browsers revalidate (cheap, by ETag).
    headers.set("cache-control", "no-cache");
    headers.set("x-content-type-options", "nosniff");

    if (ext === "html") {
      // The rewritten page is not the stored object: no ETag (pages are small, sent in full).
      headers.delete("etag");
      const res = new Response(request.method === "HEAD" ? null : obj.body, { headers });
      if (request.method === "HEAD") return res;
      return new HTMLRewriter()
        .on("head", { element: (e) => e.prepend(`<script src="${SAVE_PATH}"></script>`, { html: true }) })
        .transform(res);
    }

    if (!("body" in obj)) return new Response(null, { status: 304, headers });
    return new Response(request.method === "HEAD" ? null : obj.body, { headers });
  },
};
