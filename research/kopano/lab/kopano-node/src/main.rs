//! kopanod: the Kopano service for this computer.
//!
//!   kopanod init                 make this computer's device key (kept in the OS key store)
//!   kopanod id                   print this computer's device id
//!   kopanod run                  run until stopped
//!   kopanod autostart on|off     start (or stop starting) kopanod when this user signs in
//!   kopanod import --from <lab peer dir>    take a lab project in (lab tests; real people sign in)
//!   kopanod call --json '{"op":..}'          one request, as an app would
//!   kopanod watch                print live events, as an app panel would
//!
//! Every command takes [--home H] [--api-port P]. Defaults, shared with the apps' Kopano panels:
//! home = KOPANO_HOME, else %LOCALAPPDATA%\Kopano (Windows), ~/Library/Application Support/Kopano
//! (macOS), ~/.local/share/kopano (Linux); API port = KOPANO_API_PORT, else 47100.
//!
//! Signing in: `kopanod call --json '{"op":"sign-in"}'` (what the apps' button does; API.md).
//! ThabangVision's address: KOPANO_TV_URL, else <home>/server.json, else the local test server
//! (tv.rs).

use anyhow::{bail, Context, Result};
use kopano_core::identity::DeviceKey;
use kopano_node::{api_token, import_peer_dir, keystore, serve_api_on, share, signin, unlinked, Node};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn default_home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("KOPANO_HOME").filter(|h| !h.is_empty()) {
        return Ok(h.into());
    }
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from).with_context(|| format!("{k} is not set"));
    Ok(if cfg!(windows) {
        env("LOCALAPPDATA")?.join("Kopano")
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support/Kopano")
    } else {
        std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).map_or_else(|| env("HOME").map(|h| h.join(".local/share")), Ok)?.join("kopano")
    })
}

/// A new device key, straight into the OS key store where there is one (keystore.rs).
fn init(home: &Path) -> Result<String> {
    std::fs::create_dir_all(home.join("projects"))?;
    let f = home.join("device.key");
    if !f.exists() {
        std::fs::write(&f, DeviceKey::generate().to_bytes())?;
        if let Err(e) = keystore::protect(home) {
            eprintln!("[kopanod] {e:#}");
        }
    }
    Ok(DeviceKey::from_bytes(keystore::load(home)?).public_hex())
}

/// The Run value people see under Startup apps (Windows): never the protocol's name.
const RUN_VALUE: &str = "ThabangVision collaboration";
/// The Run value name before the installers (S4); `autostart` removes it either way.
const OLD_RUN_VALUE: &str = "Kopano";

/// Start at sign-in for this user only, without admin rights: the Run key on Windows, a
/// LaunchAgent on macOS, a systemd user unit on Linux.
fn autostart(on: bool, home: &Path, port: u16) -> Result<()> {
    let exe = std::env::current_exe()?;
    let cmdline = format!("\"{}\" run --home \"{}\" --api-port {port}", exe.display(), home.display());
    let run = |c: &mut std::process::Command| -> Result<()> {
        // the installed kopanod has no console of its own (S4): keep reg.exe from opening one
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(c, 0x0800_0000); // CREATE_NO_WINDOW
        let out = c.output()?;
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    };
    if cfg!(windows) {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let _ = run(std::process::Command::new("reg").args(["delete", key, "/v", OLD_RUN_VALUE, "/f"]));
        if on {
            run(std::process::Command::new("reg").args(["add", key, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &cmdline, "/f"]))
        } else {
            // already off is fine (an uninstaller may ask twice)
            let _ = run(std::process::Command::new("reg").args(["delete", key, "/v", RUN_VALUE, "/f"]));
            Ok(())
        }
    } else if cfg!(target_os = "macos") {
        let plist = PathBuf::from(std::env::var("HOME")?).join("Library/LaunchAgents/org.ubunye.kopano.plist");
        if on {
            std::fs::create_dir_all(plist.parent().context("plist folder")?)?;
            std::fs::write(&plist, format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>org.ubunye.kopano</string>\n<key>ProgramArguments</key><array><string>{}</string><string>run</string><string>--home</string><string>{}</string><string>--api-port</string><string>{port}</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n</dict></plist>\n",
                exe.display(), home.display()))?;
            run(std::process::Command::new("launchctl").args(["load", "-w"]).arg(&plist))
        } else {
            let _ = std::process::Command::new("launchctl").args(["unload", "-w"]).arg(&plist).output();
            let _ = std::fs::remove_file(&plist);
            Ok(())
        }
    } else {
        let unit = PathBuf::from(std::env::var("HOME")?).join(".config/systemd/user/kopano.service");
        if on {
            std::fs::create_dir_all(unit.parent().context("unit folder")?)?;
            std::fs::write(&unit, format!("[Unit]\nDescription=ThabangVision collaboration\n\n[Service]\nExecStart={cmdline}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n"))?;
            run(std::process::Command::new("systemctl").args(["--user", "enable", "--now", "kopano.service"]))
        } else {
            let _ = std::process::Command::new("systemctl").args(["--user", "disable", "--now", "kopano.service"]).output();
            let _ = std::fs::remove_file(&unit);
            Ok(())
        }
    }
}

/// The service until it is stopped. Each round runs on its own runtime: not signed in (the apps
/// get answers, nothing goes online), then online until stopped or signed out. Signing out ends
/// the round and dropping its runtime ends every task of that identity (watch loops, app
/// bindings), so the next round starts clean, back in the not-signed-in state.
fn run(home: &Path, port: u16) -> Result<()> {
    let mut first = true;
    loop {
        let rt = tokio::runtime::Runtime::new()?;
        let again = rt.block_on(round(home, port, first));
        rt.shutdown_timeout(Duration::from_secs(5));
        if !again? {
            return Ok(());
        }
        first = false;
    }
}

/// One round of `run`. True: signed out, start again.
async fn round(home: &Path, port: u16, first: bool) -> Result<bool> {
    // The port first: a second kopanod for this computer stops here, before it could go online
    // with the same identity (S4: the app and start-at-login may both start one). After a
    // sign-out this process takes its own port back.
    let mut tries = if first { 1 } else { 50 };
    let listener = loop {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => break l,
            Err(_) if tries > 1 => {
                tries -= 1;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => return Err(e).with_context(|| format!("kopanod is already running on this computer (port {port} is taken)")),
        }
    };
    init(home)?;
    let token = api_token(home)?; // before going online, so an app can read it straight away
    // not signed in yet (a fresh install, or signed out): answer the apps, stay offline
    if !home.join("cert.json").exists() {
        tokio::select! {
            r = unlinked::serve_until_signed_in(home, &listener, token) => r?,
            _ = tokio::signal::ctrl_c() => return Ok(false),
        }
    }
    let node = Node::open(home)?;
    let router = node.start().await?;
    eprintln!("[kopanod] {} online as {}, {} project(s), app port {port}", node.device.cert.body.name, &node.device.id()[..8], node.projects.read().unwrap_or_else(|e| e.into_inner()).len());
    tokio::spawn(signin::keep_fresh(node.clone()));
    share::spawn_mirror(node.clone());
    let again = tokio::select! {
        r = serve_api_on(node.clone(), &listener) => { r?; false }
        _ = node.stop.notified() => true,
        _ = tokio::signal::ctrl_c() => false,
    };
    drop(listener);
    router.shutdown().await?;
    node.close().await;
    if again {
        eprintln!("[kopanod] signed out");
    }
    Ok(again)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let home = match arg(&args, "--home") {
        Some(h) => PathBuf::from(h),
        None => default_home()?,
    };
    let port: u16 = match arg(&args, "--api-port").or_else(|| std::env::var("KOPANO_API_PORT").ok().filter(|p| !p.is_empty())) {
        Some(p) => p.parse().context("API port")?,
        None => 47100,
    };
    match args.get(1).map(String::as_str) {
        Some("init") | Some("id") => {
            println!("{}", init(&home)?);
            Ok(())
        }
        Some("autostart") => {
            let on = match args.get(2).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => bail!("kopanod autostart on|off"),
            };
            autostart(on, &home, port)?;
            println!("{{\"autostart\":{on}}}");
            Ok(())
        }
        Some("run") => run(&home, port),
        Some("import") => {
            let pid = import_peer_dir(&home, &PathBuf::from(arg(&args, "--from").context("--from")?))?;
            println!("{{\"imported\":\"{pid}\"}}");
            Ok(())
        }
        Some(op @ ("call" | "watch")) => tokio::runtime::Runtime::new()?.block_on(async {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let token = api_token(&home)?;
            let mut req: serde_json::Value = if op == "call" { serde_json::from_str(&arg(&args, "--json").context("--json")?)? } else { serde_json::json!({"op": "subscribe"}) };
            req["token"] = token.into();
            let c = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.context("kopanod is not running")?;
            let (r, mut w) = c.into_split();
            w.write_all(format!("{req}\n").as_bytes()).await?;
            let mut lines = BufReader::new(r).lines();
            while let Some(l) = lines.next_line().await? {
                println!("{l}");
                if op == "call" {
                    break;
                }
            }
            Ok(())
        }),
        _ => bail!("usage: kopanod init|id|run|autostart|import|call|watch [--home H] [--api-port P] (see the file header)"),
    }
}
