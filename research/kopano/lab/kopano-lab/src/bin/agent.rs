//! E15 test helper for a second real device (a laptop at home, then on a phone hotspot).
//!
//! On the laptop: `agent --dir bob --key agent.key --owner <hex device key of the test owner>`.
//! It keeps Bob's Kopano peer online (`peer serve`) and accepts instructions over iroh, ALPN
//! `kopano/lab-agent/0`, from ONE device only: the owner's (checked against the TLS-authenticated
//! endpoint id). It can only run a fixed list of `peer` subcommands on Bob's folder. No shell, no
//! other files. Close the window and it stops.
//!
//! On the owner's PC: `agent call --dir alice --to <agent id hex> --json '<request>'`.
//! Requests: {"op":"info"} | {"op":"serve-addr"} | {"op":"serve","relay_only":bool,"env":{..}}
//!           | {"op":"run","args":[..],"files":{"name.json":"content"},"env":{..},"timeout_s":N}

#[path = "../keystore.rs"]
#[allow(dead_code)]
mod keystore;
use anyhow::{anyhow, bail, Context, Result};
use iroh::endpoint::{presets, Connection};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

const ALPN: &[u8] = b"kopano/lab-agent/0";
const ALLOWED: [&str; 7] = ["pull", "fetch-media", "edit", "status", "verify", "hash-of", "render"];

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn now() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{:02}:{:02}:{:02}", (s / 3600 + 2) % 24, s / 60 % 60, s % 60)
}

async fn write_frame(s: &mut iroh::endpoint::SendStream, b: &[u8]) -> Result<()> {
    s.write_all(&(b.len() as u32).to_le_bytes()).await?;
    s.write_all(b).await?;
    Ok(())
}
async fn read_frame(r: &mut iroh::endpoint::RecvStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let n = u32::from_le_bytes(len) as usize;
    if n > 4 << 20 {
        bail!("request too large");
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

struct State {
    peer_exe: PathBuf,
    dir: PathBuf,
    owner: [u8; 32],
    serve: Mutex<Option<tokio::process::Child>>,
    serve_cfg: Mutex<(bool, BTreeMap<String, String>)>,
    /// Held while the peer is being stopped, used or started, so steps never overlap.
    busy: Mutex<()>,
}

impl State {
    async fn start_serve(&self) -> Result<()> {
        let (relay_only, env) = self.serve_cfg.lock().await.clone();
        let _ = std::fs::remove_file(self.dir.join("addr.json"));
        let mut c = tokio::process::Command::new(&self.peer_exe);
        let errlog = self.dir.join("serve-errors.log");
        c.arg("serve").arg("--dir").arg(&self.dir).envs(&env).stdout(std::process::Stdio::null()).stderr(std::fs::File::create(&errlog)?).kill_on_drop(true);
        if relay_only {
            c.arg("--relay-only");
        }
        let child = c.spawn().context("start peer serve")?;
        *self.serve.lock().await = Some(child);
        let t = Instant::now();
        let mut told = 0;
        while !self.dir.join("addr.json").exists() {
            if let Some(ch) = self.serve.lock().await.as_mut() {
                if let Ok(Some(st)) = ch.try_wait() {
                    let why = std::fs::read_to_string(&errlog).unwrap_or_default();
                    bail!("the test peer stopped ({st}): {}", why.trim());
                }
            }
            let s = t.elapsed().as_secs();
            if s > 180 {
                bail!("the test peer could not get online within 3 minutes; check the internet connection and start again");
            }
            if s / 30 > told {
                told = s / 30;
                println!("{}  still connecting ({s} s)...", now());
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        Ok(())
    }
    async fn stop_serve(&self) {
        if let Some(mut c) = self.serve.lock().await.take() {
            let _ = c.kill().await;
            let _ = c.wait().await;
        }
    }

    /// Restart the peer without making the caller wait (its result shows in the window).
    fn restart_in_background(self: &Arc<Self>) {
        let st = self.clone();
        tokio::spawn(async move {
            let _g = st.busy.lock().await;
            match st.start_serve().await {
                Ok(()) => println!("{}  test peer back online", now()),
                Err(e) => println!("{}  test peer could not restart: {e:#}", now()),
            }
        });
    }

    async fn handle(self: &Arc<Self>, req: Value) -> Result<Value> {
        match req.get("op").and_then(Value::as_str).unwrap_or("") {
            "info" => Ok(json!({
                "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
                "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
                "agent": env!("CARGO_PKG_VERSION"),
            })),
            "say" => {
                // A short message from the test owner, shown in the window.
                let text: String = req.get("text").and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).take(300).collect();
                println!();
                println!("  ==================================================");
                println!("  Message from the test owner's PC:");
                println!("  {text}");
                println!("  ==================================================");
                println!();
                Ok(json!({"shown": text}))
            }
            "locate" => {
                // "Find my device": the laptop works out its own position. Shown in the window every time.
                println!("{}  LOCATION requested by the test owner's PC", now());
                let ps = r#"
Add-Type -AssemblyName System.Device
$w = New-Object System.Device.Location.GeoCoordinateWatcher('High')
$w.Start()
$t = 0
while (($w.Status -ne 'Ready') -and ($w.Permission -ne 'Denied') -and ($t -lt 150)) { Start-Sleep -Milliseconds 200; $t++ }
$c = $w.Position.Location
if ($c.IsUnknown) { $lat = 'null'; $lon = 'null'; $acc = 'null' } else { $lat = $c.Latitude; $lon = $c.Longitude; $acc = $c.HorizontalAccuracy }
'{"status":"' + $w.Status + '","permission":"' + $w.Permission + '","lat":' + $lat + ',"lon":' + $lon + ',"accuracy_m":' + $acc + '}'
"#;
                let windows_loc = if cfg!(windows) {
                    let o = tokio::time::timeout(
                        std::time::Duration::from_secs(45),
                        tokio::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", ps]).output(),
                    )
                    .await;
                    match o {
                        Ok(Ok(o)) => {
                            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
                            serde_json::from_str::<Value>(&t).unwrap_or(json!({"raw": t, "stderr": tail(&String::from_utf8_lossy(&o.stderr), 400)}))
                        }
                        Ok(Err(e)) => json!({"error": e.to_string()}),
                        Err(_) => json!({"error": "timed out"}),
                    }
                } else {
                    json!({"error": "not available on this system"})
                };
                let wifi = if cfg!(windows) {
                    match tokio::process::Command::new("netsh").args(["wlan", "show", "networks", "mode=bssid"]).output().await {
                        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                        Err(e) => format!("error: {e}"),
                    }
                } else {
                    String::new()
                };
                println!("{}  location sent", now());
                Ok(json!({"windows_location": windows_loc, "wifi_scan": tail(&wifi, 20000)}))
            }
            "serve-addr" => {
                let _g = self.busy.lock().await; // wait for a restart in progress
                Ok(json!({"addr": std::fs::read_to_string(self.dir.join("addr.json")).unwrap_or_default()}))
            }
            "serve" => {
                let relay_only = req.get("relay_only").and_then(Value::as_bool).unwrap_or(false);
                let env = env_of(&req)?;
                *self.serve_cfg.lock().await = (relay_only, env);
                let _g = self.busy.lock().await;
                self.stop_serve().await;
                let t = Instant::now();
                self.start_serve().await?;
                Ok(json!({"online_ms": t.elapsed().as_millis() as u64}))
            }
            "run" => {
                let args: Vec<String> = req.get("args").and_then(Value::as_array).context("args")?.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect();
                let sub = args.first().context("no subcommand")?;
                if !ALLOWED.contains(&sub.as_str()) {
                    bail!("not allowed: {sub}");
                }
                // Files go only into Bob's folder, plain names only.
                if let Some(files) = req.get("files").and_then(Value::as_object) {
                    for (name, content) in files {
                        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') || !name.ends_with(".json") {
                            bail!("bad file name {name}");
                        }
                        std::fs::write(self.dir.join(name), content.as_str().unwrap_or(""))?;
                    }
                }
                // `--from x.json` refers to a file in Bob's folder.
                let mut full = vec![sub.clone(), "--dir".into(), self.dir.display().to_string()];
                let mut it = args.iter().skip(1);
                while let Some(a) = it.next() {
                    full.push(a.clone());
                    if a == "--from" {
                        let f = it.next().context("--from needs a file")?;
                        full.push(self.dir.join(Path::new(f).file_name().context("bad --from")?).display().to_string());
                    }
                }
                // A received file may only be saved as <package>/received/<plain name>.
                if let Some(name) = req.get("save_as").and_then(Value::as_str) {
                    if !(sub == "fetch-media" || sub == "render") || name.is_empty() || name.len() > 100 || name.contains("..")
                        || !name.chars().all(|c| c.is_ascii_alphanumeric() || " ._-".contains(c))
                    {
                        bail!("bad save_as name");
                    }
                    let root = self.dir.parent().and_then(Path::parent).context("package folder")?;
                    full.push("--out".into());
                    full.push(root.join("received").join(name).display().to_string());
                    println!("{}  receiving a file: received/{name}", now());
                }
                let timeout = req.get("timeout_s").and_then(Value::as_u64).unwrap_or(600);
                let g = self.busy.lock().await;
                self.stop_serve().await;
                let t = Instant::now();
                let out = tokio::time::timeout(
                    std::time::Duration::from_secs(timeout),
                    tokio::process::Command::new(&self.peer_exe).args(&full).envs(env_of(&req)?).kill_on_drop(true).output(),
                )
                .await;
                let ms = t.elapsed().as_millis() as u64;
                drop(g);
                self.restart_in_background();
                let (code, stdout, stderr) = match out {
                    Ok(Ok(o)) => (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string()),
                    Ok(Err(e)) => (-1, String::new(), e.to_string()),
                    Err(_) => (-2, String::new(), format!("stopped after {timeout} s")),
                };
                Ok(json!({"code": code, "ms": ms, "stdout": stdout, "stderr": tail(&stderr, 2000)}))
            }
            other => bail!("unknown op {other:?}"),
        }
    }
}

fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}

fn env_of(req: &Value) -> Result<BTreeMap<String, String>> {
    let mut m = BTreeMap::new();
    if let Some(e) = req.get("env").and_then(Value::as_object) {
        for (k, v) in e {
            if k != "KOPANO_RELAY_URL" {
                bail!("only KOPANO_RELAY_URL may be set");
            }
            m.insert(k.clone(), v.as_str().unwrap_or("").to_string());
        }
    }
    Ok(m)
}

#[derive(Clone)]
struct Handler(Arc<State>);
impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Handler")
    }
}
impl ProtocolHandler for Handler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if *conn.remote_id().as_bytes() != self.0.owner {
            println!("{}  refused a connection from an unknown device", now());
            conn.close(1u32.into(), b"not the test owner");
            return Ok(());
        }
        if let Err(e) = self.serve(conn).await {
            println!("{}  instruction failed: {e:#}", now());
        }
        Ok(())
    }
}
impl Handler {
    async fn serve(&self, conn: Connection) -> Result<()> {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let req: Value = serde_json::from_slice(&read_frame(&mut recv).await?)?;
        let label = match req.get("args").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str) {
            Some(s) => format!("{} {s}", req["op"].as_str().unwrap_or("")),
            None => req["op"].as_str().unwrap_or("").to_string(),
        };
        println!("{}  test step: {label}", now());
        let reply = match self.0.handle(req).await {
            Ok(v) => json!({"ok": true, "result": v}),
            Err(e) => json!({"ok": false, "error": format!("{e:#}")}),
        };
        println!("{}  done: {}", now(), if reply["ok"].as_bool() == Some(true) { "ok" } else { "error" });
        write_frame(&mut send, &serde_json::to_vec(&reply)?).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }
}

async fn run_agent(args: &[String]) -> Result<()> {
    let dir = std::path::absolute(PathBuf::from(arg(args, "--dir").context("--dir")?))?;
    let key: [u8; 32] = std::fs::read(arg(args, "--key").context("--key")?)?.try_into().map_err(|_| anyhow!("bad agent key"))?;
    let owner: [u8; 32] = hex::decode(arg(args, "--owner").context("--owner")?)?.try_into().map_err(|_| anyhow!("bad owner key"))?;
    let peer_exe = std::env::current_exe()?.with_file_name(if cfg!(windows) { "peer.exe" } else { "peer" });
    let state = Arc::new(State { peer_exe, dir, owner, serve: Mutex::new(None), serve_cfg: Mutex::new((false, BTreeMap::new())), busy: Mutex::new(()) });
    println!("Kopano test helper. Keep this window open during the test; close it to stop.");
    println!("{}  starting the test peer...", now());
    state.start_serve().await?;
    let ep = Endpoint::builder(presets::N0).secret_key(SecretKey::from_bytes(&key)).alpns(vec![ALPN.to_vec()]).bind().await?;
    let router = Router::builder(ep).accept(ALPN, Handler(state.clone())).spawn();
    router.endpoint().online().await;
    println!("{}  online. Test id: {}", now(), hex::encode(router.endpoint().id().as_bytes()));
    println!("{}  waiting for test steps from the test owner's PC...", now());
    tokio::signal::ctrl_c().await?;
    state.stop_serve().await;
    router.shutdown().await?;
    Ok(())
}

async fn call(args: &[String]) -> Result<()> {
    let dir = PathBuf::from(arg(args, "--dir").context("--dir")?);
    let key: [u8; 32] = keystore::load(&dir)?;
    let to: [u8; 32] = hex::decode(arg(args, "--to").context("--to")?)?.try_into().map_err(|_| anyhow!("bad id"))?;
    let req: Value = serde_json::from_str(&arg(args, "--json").context("--json")?)?;
    let ep = Endpoint::builder(presets::N0).secret_key(SecretKey::from_bytes(&key)).bind().await?;
    let t = Instant::now();
    let id = iroh::PublicKey::from_bytes(&to)?;
    let conn = ep.connect(EndpointAddr::from_parts(id, std::iter::empty()), ALPN).await?;
    let connect_ms = t.elapsed().as_millis();
    let path = conn.paths().iter().find(|p| p.is_selected()).map(|p| format!("{:?}", p.remote_addr())).unwrap_or_else(|| "unknown".into());
    let (mut send, mut recv) = conn.open_bi().await?;
    write_frame(&mut send, &serde_json::to_vec(&req)?).await?;
    send.finish()?;
    let reply: Value = serde_json::from_slice(&read_frame(&mut recv).await?)?;
    conn.close(0u32.into(), b"done");
    println!("{}", json!({"control_connect_ms": connect_ms, "control_path": path, "total_ms": t.elapsed().as_millis() as u64, "reply": reply}));
    ep.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let r = if args.get(1).map(String::as_str) == Some("call") { call(&args).await } else { run_agent(&args).await };
    if let Err(e) = &r {
        println!("{}  stopped: {e:#}", now());
        if args.get(1).map(String::as_str) != Some("call") {
            println!("Press Enter to close.");
            let mut s = String::new();
            let _ = std::io::stdin().read_line(&mut s);
        }
    }
    r
}
