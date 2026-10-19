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
//!           | {"op":"start", same fields as run} -> {"job":N} | {"op":"job","job":N} | {"op":"net"}
//!
//! v8 (E37 lessons): a long step no longer holds the control connection open. `start` returns at
//! once and the step keeps running on the laptop whatever happens to the link; `job` reads its
//! state and output so far. `net` reports what decides whether a teammate on the same Wi-Fi can
//! reach this laptop: the network type Windows gave the connection and the firewall rules for this
//! package's peer.exe.

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
const HELPER: &str = "v8";
// v7 adds only commands that cannot read or overwrite arbitrary files on the laptop
const ALLOWED: [&str; 10] = ["pull", "fetch-media", "edit", "status", "verify", "hash-of", "render", "fetch-swarm", "fetch-folder", "import-bundle"];

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
    jobs: std::sync::Mutex<BTreeMap<u64, Job>>,
}

struct Job {
    label: String,
    started: Instant,
    /// set when the step has the peer to itself (an earlier step may still be running)
    running: Option<Instant>,
    /// exit code and run time once finished; -2 = stopped at its time limit, -1 = could not start
    done: Option<(i32, u64)>,
    out: PathBuf,
    err: PathBuf,
}

/// A step ready to run: the full `peer` command line, its settings and time limit.
struct Step {
    label: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    timeout_s: u64,
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

    /// Check a step request and turn it into a `peer` command line on Bob's folder.
    fn prepare(&self, req: &Value) -> Result<Step> {
        let args: Vec<String> = req.get("args").and_then(Value::as_array).context("args")?.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect();
        let sub = args.first().context("no subcommand")?;
        if !ALLOWED.contains(&sub.as_str()) {
            bail!("not allowed: {sub}");
        }
        // Files go only into Bob's folder, plain names only.
        if let Some(files) = req.get("files").and_then(Value::as_object) {
            for (name, content) in files {
                if !plain_json_name(name) {
                    bail!("bad file name {name}");
                }
                std::fs::write(self.dir.join(name), content.as_str().unwrap_or(""))?;
            }
        }
        // `--from x.json` (v8: also `--from a.json,b.json` for several teammates) names files in
        // Bob's folder only.
        let mut full = vec![sub.clone(), "--dir".into(), self.dir.display().to_string()];
        let mut it = args.iter().skip(1);
        while let Some(a) = it.next() {
            full.push(a.clone());
            if a == "--from" {
                let list = it.next().context("--from needs a file")?;
                let mut paths = vec![];
                for f in list.split(',') {
                    if !plain_json_name(f) {
                        bail!("bad --from name {f}");
                    }
                    paths.push(self.dir.join(f).display().to_string());
                }
                full.push(paths.join(","));
            }
        }
        // A received file may only be saved as <package>/received/<plain name>.
        if let Some(name) = req.get("save_as").and_then(Value::as_str) {
            if !(sub == "fetch-media" || sub == "render") || name.is_empty() || name.len() > 100 || name.contains("..")
                || !name.chars().all(|c| c.is_ascii_alphanumeric() || " ._-".contains(c))
            {
                bail!("bad save_as name");
            }
            full.push("--out".into());
            full.push(self.root()?.join("received").join(name).display().to_string());
            println!("{}  receiving a file: received/{name}", now());
        }
        Ok(Step { label: sub.clone(), args: full, env: env_of(req)?, timeout_s: req.get("timeout_s").and_then(Value::as_u64).unwrap_or(600) })
    }

    fn root(&self) -> Result<PathBuf> {
        Ok(self.dir.parent().and_then(Path::parent).context("package folder")?.to_path_buf())
    }

    /// Run a step in the background, one at a time; output goes to <package>/jobs/<n>.out, .err.
    fn start_job(self: &Arc<Self>, step: Step) -> u64 {
        let jobs_dir = self.root().map(|r| r.join("jobs")).unwrap_or_else(|_| self.dir.join("jobs"));
        let _ = std::fs::create_dir_all(&jobs_dir);
        let id = {
            let mut jobs = self.jobs.lock().unwrap();
            let id = jobs.keys().next_back().map_or(1, |k| k + 1);
            let (out, err) = (jobs_dir.join(format!("{id}.out")), jobs_dir.join(format!("{id}.err")));
            jobs.insert(id, Job { label: step.label.clone(), started: Instant::now(), running: None, done: None, out, err });
            id
        };
        let st = self.clone();
        tokio::spawn(async move {
            let g = st.busy.lock().await;
            st.stop_serve().await;
            let (out, err) = {
                let mut jobs = st.jobs.lock().unwrap();
                let j = jobs.get_mut(&id).unwrap();
                j.running = Some(Instant::now());
                (j.out.clone(), j.err.clone())
            };
            println!("{}  step {id} ({}) running", now(), step.label);
            let t = Instant::now();
            let code = st.run_step(&step, &out, &err).await;
            let ms = t.elapsed().as_millis() as u64;
            st.jobs.lock().unwrap().get_mut(&id).unwrap().done = Some((code, ms));
            println!("{}  step {id} ({}) finished: {}", now(), step.label, if code == 0 { "ok".to_string() } else { format!("code {code}") });
            drop(g);
            st.restart_in_background();
        });
        id
    }

    async fn run_step(&self, step: &Step, out: &Path, err: &Path) -> i32 {
        let note = |text: String| {
            use std::io::Write;
            let _ = std::fs::OpenOptions::new().create(true).append(true).open(err).and_then(|mut f| writeln!(f, "{text}"));
        };
        let (o, e) = match (std::fs::File::create(out), std::fs::File::create(err)) {
            (Ok(o), Ok(e)) => (o, e),
            _ => return -1,
        };
        let mut child = match tokio::process::Command::new(&self.peer_exe).args(&step.args).envs(&step.env).stdout(o).stderr(e).kill_on_drop(true).spawn() {
            Ok(c) => c,
            Err(e) => {
                note(format!("could not start: {e}"));
                return -1;
            }
        };
        match tokio::time::timeout(std::time::Duration::from_secs(step.timeout_s), child.wait()).await {
            Ok(Ok(s)) => s.code().unwrap_or(-1),
            Ok(Err(e)) => {
                note(e.to_string());
                -1
            }
            Err(_) => {
                let _ = child.kill().await;
                note(format!("stopped after {} s", step.timeout_s));
                -2
            }
        }
    }

    fn job_state(&self, id: u64, tail_n: usize) -> Result<Value> {
        let jobs = self.jobs.lock().unwrap();
        let j = jobs.get(&id).with_context(|| format!("no step {id}"))?;
        let read = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
        let state = if j.done.is_some() { "done" } else if j.running.is_some() { "running" } else { "waiting" };
        let mut v = json!({
            "job": id, "label": j.label, "state": state,
            "since_start_ms": j.started.elapsed().as_millis() as u64,
            "running_ms": j.running.map(|r| r.elapsed().as_millis() as u64),
            "stdout": tail(&read(&j.out), tail_n), "stderr": tail(&read(&j.err), tail_n.min(4000)),
        });
        if let Some((code, ms)) = j.done {
            v["code"] = json!(code);
            v["ms"] = json!(ms);
        }
        Ok(v)
    }

    /// What decides whether a teammate on the same Wi-Fi can reach this laptop. Reads settings
    /// only; changes nothing.
    async fn net(&self) -> Value {
        println!("{}  network check requested (reads settings only)", now());
        let exe = self.peer_exe.display().to_string();
        let mut cmd = if cfg!(windows) {
            let mut c = tokio::process::Command::new("powershell");
            c.args(["-NoProfile", "-NonInteractive", "-Command", &NET_CHECK.replace("@EXE@", &exe.replace('\'', "''"))]);
            c
        } else if cfg!(target_os = "macos") {
            let mut c = tokio::process::Command::new("/bin/sh");
            c.args(["-c", NET_CHECK_MAC, "net-check", &exe]);
            c
        } else {
            return json!({"error": "not available on this system"});
        };
        let o = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output()).await;
        match o {
            Ok(Ok(o)) => {
                let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
                serde_json::from_str::<Value>(&t).unwrap_or(json!({"raw": tail(&t, 4000), "stderr": tail(&String::from_utf8_lossy(&o.stderr), 400)}))
            }
            Ok(Err(e)) => json!({"error": e.to_string()}),
            Err(_) => json!({"error": "timed out"}),
        }
    }

    async fn handle(self: &Arc<Self>, req: Value) -> Result<Value> {
        match req.get("op").and_then(Value::as_str).unwrap_or("") {
            "info" => Ok(json!({
                "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
                "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
                "agent": env!("CARGO_PKG_VERSION"), "helper": HELPER, "allowed": ALLOWED,
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
                // the v7 form, kept for old drivers: start the step and wait for it here
                let id = self.start_job(self.prepare(&req)?);
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let v = self.job_state(id, usize::MAX)?;
                    if v["state"] == "done" {
                        return Ok(json!({"code": v["code"], "ms": v["ms"], "stdout": v["stdout"], "stderr": v["stderr"]}));
                    }
                }
            }
            "start" => {
                let id = self.start_job(self.prepare(&req)?);
                Ok(json!({"job": id}))
            }
            "job" => {
                let id = req.get("job").and_then(Value::as_u64).context("job")?;
                let tail_n = req.get("tail").and_then(Value::as_u64).unwrap_or(4000) as usize;
                self.job_state(id, tail_n)
            }
            "net" => Ok(self.net().await),
            other => bail!("unknown op {other:?}"),
        }
    }
}

fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}

/// Network type, IPv4 addresses and the firewall rules for this package's peer.exe (read only).
const NET_CHECK: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$profiles = @(Get-NetConnectionProfile | ForEach-Object { [pscustomobject]@{ alias = $_.InterfaceAlias; category = [string]$_.NetworkCategory } })
$ips = @(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } | ForEach-Object { $_.IPAddress + '/' + $_.PrefixLength })
$exe = '@EXE@'
$rules = @(Get-NetFirewallApplicationFilter | Where-Object { $_.Program -eq $exe } | ForEach-Object {
  $r = $_ | Get-NetFirewallRule
  $a = $r | Get-NetFirewallAddressFilter
  $p = $r | Get-NetFirewallPortFilter
  [pscustomobject]@{ name = $r.DisplayName; direction = [string]$r.Direction; action = [string]$r.Action; enabled = [string]$r.Enabled; profiles = [string]$r.Profile; protocol = [string]$p.Protocol; remote = [string]$a.RemoteAddress }
})
$fw = @(Get-NetFirewallProfile | ForEach-Object { [pscustomobject]@{ name = $_.Name; enabled = [string]$_.Enabled; inbound = [string]$_.DefaultInboundAction } })
[pscustomobject]@{ peer_exe = $exe; profiles = $profiles; ipv4 = $ips; rules = $rules; firewall = $fw } | ConvertTo-Json -Depth 4 -Compress
"#;

/// The same on a Mac: application firewall state and whether peer ($1) is blocked (read only).
const NET_CHECK_MAC: &str = r#"
fw=/usr/libexec/ApplicationFirewall/socketfilterfw
q() { printf '%s' "$1" | tr -d '\r' | tr '\n' ' ' | sed 's/"/'"'"'/g'; }
ips=$(ifconfig 2>/dev/null | awk '/inet / && $2 != "127.0.0.1" {print $2}' | tr '\n' ' ')
printf '{"peer_exe":"%s","firewall_global":"%s","stealth":"%s","block_all":"%s","peer_app":"%s","ipv4":"%s"}\n' \
  "$(q "$1")" "$(q "$($fw --getglobalstate 2>&1)")" "$(q "$($fw --getstealthmode 2>&1)")" \
  "$(q "$($fw --getblockall 2>&1)")" "$(q "$($fw --getappblocked "$1" 2>&1)")" "$(q "$ips")"
"#;

fn plain_json_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && !name.contains("..") && name.ends_with(".json") && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// Settings the test owner may change remotely, each limited to known values (v8: LAN-only and
/// congestion control join the relay, so E37 runs the same settings on every machine).
fn env_of(req: &Value) -> Result<BTreeMap<String, String>> {
    let mut m = BTreeMap::new();
    if let Some(e) = req.get("env").and_then(Value::as_object) {
        for (k, v) in e {
            let v = v.as_str().unwrap_or("");
            let ok = match k.as_str() {
                "KOPANO_RELAY_URL" => v.starts_with("https://") && v.len() < 200 && !v.chars().any(char::is_whitespace),
                "KOPANO_LAN_ONLY" => v == "0" || v == "1",
                "KOPANO_CC" => ["default", "bbr", "cubic", "reno"].contains(&v),
                _ => false,
            };
            if !ok {
                bail!("setting not allowed: {k}={v}");
            }
            m.insert(k.clone(), v.to_string());
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
    let state = Arc::new(State { peer_exe, dir, owner, serve: Mutex::new(None), serve_cfg: Mutex::new((false, BTreeMap::new())), busy: Mutex::new(()), jobs: std::sync::Mutex::new(BTreeMap::new()) });
    println!("Kopano test helper {HELPER}. Keep this window open during the test; close it to stop.");
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
    let conn = tokio::time::timeout(std::time::Duration::from_secs(60), ep.connect(EndpointAddr::from_parts(id, std::iter::empty()), ALPN))
        .await
        .map_err(|_| anyhow!("the laptop did not answer within 60 s"))??;
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
