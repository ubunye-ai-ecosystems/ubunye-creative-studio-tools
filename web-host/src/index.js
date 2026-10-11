// Serves the browser builds of Ubunye Light, Photo and Film from the R2 bucket
// ubunye-creative-tools (keys web/<app>/<path>). The builds are static sites, but their
// .wasm files are bigger than the 25 MiB per-file limit of Pages and Workers assets.
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

    let path = decodeURIComponent(url.pathname);
    if (path.endsWith("/")) path += "index.html";
    if (path.split("/").includes("..")) return new Response("Not found", { status: 404 });

    const obj = await env.BUCKET.get(`web/${app}${path}`, { onlyIf: request.headers });
    if (!obj) return new Response("Not found", { status: 404 });

    const headers = new Headers();
    obj.writeHttpMetadata(headers);
    headers.set("etag", obj.httpEtag);
    const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
    if (TYPES[ext]) headers.set("content-type", TYPES[ext]);
    // File names stay the same between versions, so browsers revalidate (cheap, by ETag).
    headers.set("cache-control", "no-cache");
    headers.set("x-content-type-options", "nosniff");

    if (!("body" in obj)) return new Response(null, { status: 304, headers });
    return new Response(request.method === "HEAD" ? null : obj.body, { headers });
  },
};
