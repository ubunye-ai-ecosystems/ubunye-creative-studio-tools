# Windows build and launch check: LightCraft, PhotoCraft, FilmCraft

Date: 2026-10-10. Machine: Windows 11 Pro 22621, RTX 2070 SUPER 8 GB, 16 GB RAM, 16 logical CPUs.
No tracked files were changed, nothing was committed or pushed.

## Verdict

All three desktop apps and all three CLIs build in release mode on this box with the stock MSVC
toolchain, and all three GUIs open a window and stay up. The full test suites mostly pass. Every
failure found is either a Windows test harness issue (path separators, CRLF, parallel fixture or
GPU contention) or a flake; each one passes on a serial or alternate backend rerun, except the
8 LightCraft path separator tests, which are a test bug and were not rerun. No product code
failed to compile at the commit that was built. The LightCraft WebAssembly build also works.

## Toolchain

| Item | Value |
|---|---|
| rustup | 1.29.1, installed non interactively from https://win.rustup.rs/x86_64 into `T:/rust/installer` |
| RUSTUP_HOME / CARGO_HOME | `T:\rust\rustup` / `T:\rust\cargo` (PATH not modified; `--no-modify-path`) |
| Toolchain | `stable-x86_64-pc-windows-msvc`, rustc 1.99.0 (2026-09-28), cargo 1.99.0 |
| Pinning | None of the repos has `rust-toolchain(.toml)`. `rust-version` (MSRV) is 1.90 for LightCraft and 1.95 for PhotoCraft and FilmCraft, so stable 1.99 satisfies all three |
| MSVC | Already present: Visual Studio Build Tools 2019 (16.11.18), MSVC 14.29.30133, Windows SDK 10.0.19041.0. rustc found `link.exe` itself. Nothing was installed on C: |
| Extra (wasm only) | target `wasm32-unknown-unknown`, `wasm-bindgen-cli 0.2.129` (matches LightCraft's Cargo.lock) installed into `T:\rust\cargo\bin` |
| Target dirs | `CARGO_TARGET_DIR=T:/rust/targets/<app>` |
| CRAFT_FONTS_DIR | Not set. All three repos document it as optional (CJK glyphs are missing without it); builds, tests and launches all worked without it |

Build docs read: README.md, CLAUDE.md, AGENTS.md, `.cargo/config.toml` (only an `xtask` alias in each),
PhotoCraft `docs/development.md` (Windows section: MSVC toolchain + VS Build Tools; Smart App Control
can block cargo; it did not here), LightCraft `docs/web.md`, FilmCraft `docs/testing.md`.

## Commits tested

The repos were fast forwarded by a `git pull` from outside this task at about 21:51 local time,
partway through the run. Results map to commits as follows:

| App | Release build, launch, CLI | Tests |
|---|---|---|
| LightCraft | b70ae174 (clone) | b70ae174; wasm build at 964d9338 (after the pull) |
| PhotoCraft | 3e000bc1 (clone) | 3e000bc1 |
| FilmCraft | 98ba9ac (clone) | 3665378 (after the pull; the first test run was torn by the pull and thrown away, see below) |

## Release builds

Commands were run from each repo root, following the README (`cargo run --release -p <app>`).

| App | Command | Wall time | Result | Binaries (T:/rust/targets/<app>/release) |
|---|---|---|---|---|
| LightCraft | `cargo build --release -p lightcraft -p lightcraft-cli` | 574 s (9m33s; includes the first crates.io download) | OK | lightcraft.exe 53.9 MB, lightcraft-cli.exe 51.7 MB |
| PhotoCraft | `cargo build --release -p photocraft -p photocraft-cli` | 765 s (12m44s; release profile is thin LTO, 1 CGU) | OK | photocraft.exe 62.6 MB, photocraft-cli.exe 40.0 MB |
| FilmCraft | `cargo build --release -p filmcraft -p filmcraft-cli` | 411 s (6m50s) | OK | filmcraft.exe 53.0 MB, filmcraft-cli.exe 39.8 MB |

No compile errors. Builds were run one after another, never in parallel.

Note: the LightCraft build first landed in `T:\rust\targets$app` because of a quoting slip in my wrapper
script; the folder was moved to `T:\rust\targets\lightcraft` and the binaries used from there.

## GUI launch (start, wait about 12 s, check alive, kill by PID)

| App | Command | Result |
|---|---|---|
| LightCraft | `lightcraft.exe --memory` (in-memory demo, writes nothing) | Alive, window title "LightCraft", 478 MB working set. Log: `gpu: NVIDIA GeForce RTX 2070 SUPER (Dx12)` |
| PhotoCraft | `photocraft.exe` with `PHOTOCRAFT_CONFIG_DIR=T:\rust\logs\photocraft-config` | Alive, window "PhotoCraft", 260 MB. GPU canvas Bgra8Unorm, max texture 32768 |
| FilmCraft | `filmcraft.exe --demo --no-recover --data-dir T:\rust\logs\filmcraft-data` | Alive, window "FilmCraft", 331 MB. Log: `hardware decoding: Available("Media Foundation")` |

All processes were killed by PID; none are left running.

Gotcha: the FilmCraft release exe is a `windows_subsystem = "windows"` binary, so `filmcraft.exe --version`
and `--help` show a message box and wait for OK instead of printing and exiting. That is by design
(`apps/filmcraft/src/main.rs` line 63 comment) but it blocks scripted use; use `filmcraft-cli --version`.
LightCraft's and PhotoCraft's `--version` print to the console (`lightcraft 0.5.0`, `photocraft 0.6.0 (dev build)`).

### Headless rendering proof (extra)

- `lightcraft-cli snapshot --demo -o lightcraft-snapshot.png`: full UI rendered on the CPU, 1600x1000, 3.3 s, 228 frames. The image shows the Develop view with the demo library, histogram and sliders.
- `filmcraft-cli --demo render --seconds 2 --out filmcraft-frame.png`: `rendered 1920x1080 in 445.7 ms`.
- `photocraft-cli run --new '{"width":640,"height":480}' --cmd filter.blur.gaussianBlur --params '{"radius":4}' --out photocraft-new.png`: OK.
- Command registry sizes (`<cli> commands`): LightCraft 602 lines, PhotoCraft 853, FilmCraft not counted.

Output files are in `T:/rust/logs/`.

## CLI help excerpts

`lightcraft-cli --help`
```
lightcraft-cli — headless LightCraft (photo library + raw developer)
USAGE:
  lightcraft-cli run [OPTIONS] COMMAND [key=value | '{json}']… [COMMAND …]…
  lightcraft-cli mcp [OPTIONS] [FILES/FOLDERS…]        MCP server (JSON-RPC 2.0 over stdio)
  lightcraft-cli render <IN> -o <OUT> [OPTIONS]         develop one file and export it
  lightcraft-cli snapshot [OPTIONS] [FILES/FOLDERS…]    full app UI headlessly, writes PNGs
```

`photocraft-cli --help`
```
photocraft-cli: headless Photocraft
  photocraft-cli convert <in> <out> [--format <ext>] [--quality <1-100>] [--tiff-layers]
  photocraft-cli info <file> [--compact]
  photocraft-cli run (<file> | --new <json>) --cmd <id> [--params <json>] ... [--out <file>]
  photocraft-cli batch --actions <actions.json> --in <dir> --out <dir> ...
  photocraft-cli droplet <file.pcdroplet> <file-or-dir>… [--out <dir>]
  photocraft-cli commands [--json] [--filter <text>]
  photocraft-cli mcp [--bridge <127.0.0.1:port>] ...
  photocraft-cli serve [--port <port>] ...
```

`filmcraft-cli --help`
```
filmcraft-cli: drive FilmCraft from the shell (headless engine, or the live app with --bridge)
SUBCOMMANDS
  exec <id> [key=value ...]     run one command; prints its result as JSON
  run <script.jsonl | ->        run commands, one {"id","params"} per line
  commands [filter] [--json]    list commands
  describe <id> / inspect [project|sequence] / import <file>...
  export <out> [--preset name] [--format f] ...   (h264|hevc|prores|dnxhr|apv|mjpeg|mxf-op1a|...)
  render --seconds S --out f.png [--scale 0.5]
  probe <media> / bench-decode <media> / mcp
```

## Tests

Command for all three: `cargo test --workspace --no-fail-fast -j 4` (the documented `cargo test --workspace`
plus no-fail-fast; `-j 4` because the default 16 jobs ran this 16 GB box out of RAM, see below).
ffmpeg 8.1.1 (essentials build) is on PATH, so FilmCraft's ffmpeg oracle tests ran instead of skipping.

| App | Suites | Passed | Failed | Ignored | Test compile | Test run |
|---|---|---|---|---|---|---|
| LightCraft | 82 | 2525 | 8 | 26 | about 43 min total (33 min in a 16-job attempt that thrashed, then 10m16s at -j 4) | about 4 min |
| PhotoCraft | 219 | 5768 | 19 | 51 | about 59 min at -j 4 (first run hit my 50 min cap while still compiling; the rerun finished the last 8m54s) | about 9 min |
| FilmCraft | 241 reported + 1 killed | 2644 | 5 + 10 hung | 41 | 25m46s at -j 4 | about 29 min, including a 20 min hang that I killed |

Full logs: `T:/rust/logs/<app>-test.log`.

### LightCraft failures (8)

- 7 in `crates/engine/src/tests_sync.rs`: test expectations build paths with `/`, but Windows returns `\`. Test bug, Windows only. Example:
  ```
  left:  ["C:\\Users\\...\\lc-sync-local-22792\\trip\\c.png"]
  right: ["C:\\Users\\...\\lc-sync-local-22792\\trip/c.png"]
  ```
- 1 `tests_segment::a_failing_download_ends_with_an_error_and_never_blocks` (`assertion failed: again["started"] == false || again["downloading"] == false`): timing flake. A rerun of `tests_segment` passed 6 of 6.

### PhotoCraft failures (19, all in `photocraft-ui-egui` GPU canvas tests)

10 test binaries (arbitrary_rotation_preview, canvas_16f, checker_canvas, color_managed_canvas,
crop_beyond_canvas, drag_preview_canvas, full_res_preview, live_stroke_canvas, paint_bucket_all_layers,
ui_scale_canvas) failed on the default wgpu backend (DX12):
```
egui-wgpu-0.36.2\src\renderer.rs:984:17: Failed to create staging buffer for index data. Index count: 28326 ...
egui_kittest-0.36.2\src\texture_to_image.rs:59:30: called `Result::unwrap()` on an `Err` value: BufferAsyncError
crates\ui-egui\tests\arbitrary_rotation_preview.rs:49:5: the fixture must use the GPU canvas
```
They still fail when run serially (`--test-threads=1`: 12 passed, 16 failed), so it is not contention.
With `WGPU_BACKEND=vulkan` all 28 tests in those 10 binaries pass. So the egui_kittest test harness fails on DX12
on this NVIDIA box, while the app itself starts fine on its default backend.

### FilmCraft failures (5 + 10 hung)

- `filmcraft-gpu --lib`: 10 of 32 tests (e.g. `tests::gpu_matches_cpu_plan`, `blend_tests::gpu_blend_modes_match_cpu`) hung with no CPU use for over 20 minutes; I killed that one test process by PID so the rest of the suite could run. Running the same binary with `--test-threads=1` passes 32 of 32 in 10.7 s. Running it in parallel on Vulkan hangs too. This is a deadlock when many GPU tests share the device in parallel, not a backend problem.
- `filmcraft-hevc --test conformance` (2) `Os { code: 5, kind: PermissionDenied, message: "Access is denied." }` at `crates\hevc\tests\common\mod.rs:145` (`std::fs::rename(&tmp, &hevc)`), and `filmcraft-codecs --test mpeg_oracle` (1) `could not generate ts_mpeg2_mp2.ts`: parallel fixture generation racing on Windows file renames. Serial rerun: both suites pass.
- `filmcraft-codecs gop::tests::intra_frames_decode_in_parallel_once_each`: `s.decodes <= 64 + 4 * 4` exceeded under load. Flake; passed 5 of 5 on rerun.
- `xtask tests::macos_bundle_may_use_the_microphone`: checks the plist for `"...</key>\n  <true/>"`, but this clone has `core.autocrlf=true`, so the file has CRLF. Windows checkout artifact.

Torn run, thrown away: the first FilmCraft test run started at 98ba9ac. The outside `git pull` landed mid-compile and
produced `error[E0433]: cannot find module or crate arboard` / `ClipboardMedia` in `apps\filmcraft\src\main.rs`
(old manifest, new source). The clean rerun at 3665378 compiled without errors.

## WebAssembly (LightCraft only)

| Step | Time | Result |
|---|---|---|
| `rustup target add wasm32-unknown-unknown` | 8 s | OK |
| `cargo install wasm-bindgen-cli --version 0.2.129 --locked` | 185 s | OK |
| `cargo xtask web` (web profile: fat LTO, 1 CGU, opt "s" with hot crates at 3) | 190 s | OK |

Output: `T:/rust/targets/lightcraft/web/` (index.html, worker.js, lightcraft_web.js, lightcraft_web_bg.wasm, plus .gz/.br).
The wasm is 21.2 MB raw, 6.97 MB gzip, 4.57 MB brotli (no wasm-opt installed). `docs/web.md` quotes 13.5 MB / 3.1 MB
brotli, so the module has grown about 50 percent since that table was written. Not served or opened in a browser.

## Resource findings (important for anyone repeating this)

- Disk: `T:/rust` now holds **182 GB** (rustup 0.7, cargo home 0.9, lightcraft 23, photocraft 81, filmcraft 78, wasm-bindgen build 0.4). Almost all of it is debug and test artifacts with debuginfo (FilmCraft `debug/` alone is 76.7 GB). T: went from 236 GB to 39 GB free. Release-only builds are small (FilmCraft `release/` is 1.7 GB). `cargo clean` per target dir, or deleting `T:/rust/targets/*/debug`, gets it back; I left everything in place.
- RAM: the default `cargo test --workspace` (16 jobs) on 16 GB RAM drove 13 parallel `link.exe` processes into heavy paging (0.6 GB free, 44 GB committed) and stalled for 20 minutes. `-j 4` worked. Release builds at default jobs were fine.
- C: drive was not used for builds. Only test temp files (small) went to `%TEMP%`.

## Not tested

- Linux, macOS, FreeBSD, Windows arm64 and x86 builds.
- Installers (MSI, portable zip), packaging scripts in `packaging/`, code signing, notarisation, the `dist` profile.
- `cargo xtask ci` (fmt, clippy `-D warnings`, layers, assets) and the documented corpus tests (`cargo xtask corpus`, `test-corpus`) that need downloads.
- Builds with `CRAFT_FONTS_DIR` (CJK fonts), and real user media in the GUIs (only demo content was opened).
- Interactive GUI use beyond "window opens and stays up", the `--control` automation channel, MCP servers.
- The web build in a browser; PhotoCraft and FilmCraft web builds.
- NVENC / hardware encode paths beyond whatever the test suites exercised.

## Logs and artifacts

All in `T:/rust/logs/`: per step build and test logs (`<app>-build-release.log`, `<app>-test.log`, reruns, aborted runs
kept with descriptive names), GUI stderr logs, and the three headless render PNGs.
