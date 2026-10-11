// Copies a release file from GitHub's storage into the R2 bucket, so big files never pass
// through the uploader's own connection. POST JSON {url, key, size, contentType} with the
// header "x-ingest-key: <INGEST_KEY secret>". `url` must be a short-lived signed GitHub
// release-asset URL (the uploader resolves it; no GitHub token reaches this Worker).
const ALLOWED_HOSTS = ["release-assets.githubusercontent.com", "objects.githubusercontent.com"];
const KEY_RE = /^(light|photo|film)\/[0-9A-Za-z._-]+\/[0-9A-Za-z._-]+$/;

function safeEqual(a, b) {
  if (typeof a !== "string" || typeof b !== "string" || a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

export default {
  async fetch(request, env) {
    if (request.method !== "POST") return new Response("Method not allowed", { status: 405 });
    if (!env.INGEST_KEY || !safeEqual(request.headers.get("x-ingest-key"), env.INGEST_KEY)) {
      return new Response("Forbidden", { status: 403 });
    }
    let job;
    try {
      job = await request.json();
    } catch {
      return new Response("Bad JSON", { status: 400 });
    }
    const { url, key, size, contentType } = job;
    let src;
    try {
      src = new URL(url);
    } catch {
      return new Response("Bad url", { status: 400 });
    }
    if (src.protocol !== "https:" || !ALLOWED_HOSTS.includes(src.hostname)) {
      return new Response("Source not allowed", { status: 400 });
    }
    if (typeof key !== "string" || !KEY_RE.test(key) || !Number.isInteger(size) || size <= 0) {
      return new Response("Bad key or size", { status: 400 });
    }

    const res = await fetch(src.toString());
    if (!res.ok || !res.body) return new Response(`Source returned ${res.status}`, { status: 502 });
    const length = Number(res.headers.get("content-length"));
    if (length !== size) return new Response(`Size mismatch: ${length} != ${size}`, { status: 502 });

    // R2 needs a known length to stream; FixedLengthStream also errors if the bytes run short.
    const { readable, writable } = new FixedLengthStream(size);
    const pump = res.body.pipeTo(writable);
    const obj = await env.BUCKET.put(key, readable, {
      httpMetadata: { contentType: contentType || "application/octet-stream" },
    });
    await pump;
    return Response.json({ key: obj.key, size: obj.size, etag: obj.etag });
  },
};
