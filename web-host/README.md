# Browser versions host

The browser builds of Ubunye Light, Photo and Film are served from the R2 bucket
`ubunye-creative-tools` by the Cloudflare Worker `ubunye-creative-web`:

| App | Address | R2 prefix |
|---|---|---|
| Ubunye Light | https://light.tools.thabangvision.com | `web/light/` |
| Ubunye Photo | https://photo.tools.thabangvision.com | `web/photo/` |
| Ubunye Film | https://film.tools.thabangvision.com | `web/film/` |

Why a Worker and not Pages: the `.wasm` files are 25 to 43 MB, over the 25 MiB per-file
limit of Pages and Workers static assets. Cloudflare brotli-compresses them on the way out
(about a third of the size).

Desktop downloads live in the same bucket and are served at https://downloads.thabangvision.com
(R2 custom domain, no Worker).

## Update a browser version

1. Build it from the fork's `develop`:
   - Light: `cargo xtask web` (output `$CARGO_TARGET_DIR/web`)
   - Film: `cargo xtask web` (output `$CARGO_TARGET_DIR/web/dist`)
   - Photo: `trunk build --release` in `apps/photocraft-web` (output `dist/web`)
   Set `CRAFT_FONTS_DIR` to a checkout of storytold/craft-fonts at the ref the fork's release.yml pins.
2. `bash upload_web.sh <output dir> <light|photo|film>`
3. Open the address and check it starts.

## Deploy the Worker

`npx wrangler deploy` in this folder.
