//! "Sign in with ThabangVision" for this computer (09 section 3, slice S2): the device flow
//! (RFC 8628, like `gh auth login`), silent renewal, sign out.
//!
//! 1. `start`: ThabangVision gets the device's public key and answers with a short code. The
//!    browser opens at the /link page with that code; the person signs in there (2-step and all)
//!    and approves "Link this computer". The app never sees a password or a website session.
//! 2. kopanod polls with a signature by the device key until the certificate comes, checks it
//!    (signed by a key ThabangVision publishes, for this device, not expired) and writes
//!    `cert.json`. The unlinked service (unlinked.rs) sees it and goes online.
//! 3. `keep_fresh`: certificates last 30 days. When less than 7 are left (or at start-up when it
//!    has expired) the device renews by signing a fresh challenge. The new certificate is used at
//!    once, without a restart. If ThabangVision says the computer was unlinked or the account is
//!    suspended, kopanod signs out: peers refuse its old certificate when it expires.
//! 4. `sign_out`: forget the certificate (the device key stays in the OS key store, so signing in
//!    again links the same computer) and go back to the unlinked service.

use crate::tv::{self, plain_error, Tv};
use crate::{keystore, Node};
use anyhow::{bail, Context, Result};
use kopano_core::identity::{now_ms, DeviceCert, DeviceKey};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DAY_MS: u64 = 24 * 3600 * 1000;

/// Renew when less than this is left (desktop-api.md 1c: 7 days). `KOPANO_RENEW_BEFORE_MS`
/// changes it for tests (31 days renews at once).
fn renew_before_ms() -> u64 {
    std::env::var("KOPANO_RENEW_BEFORE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(7 * DAY_MS)
}

/// Where a sign-in started from this service stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Flow {
    #[default]
    Idle,
    /// waiting for the person to approve in the browser
    Waiting { code: String, link: String, until_ms: u64 },
    Failed(String),
}

pub struct SignIn {
    home: PathBuf,
    flow: Mutex<Flow>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// This computer's label on /link ("Studio PC") and its system.
pub fn computer() -> (String, String) {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Computer".into());
    let os = if cfg!(windows) { "Windows" } else if cfg!(target_os = "macos") { "macOS" } else { "Linux" };
    (name, os.into())
}

/// Open the /link page in the person's browser. Only a web address on the configured site is
/// opened, and without a shell (no command line to inject into). `KOPANO_NO_BROWSER=1` skips it
/// (tests, and machines where the app opens the page itself).
fn open_browser(url: &str, base: &str) {
    if std::env::var("KOPANO_NO_BROWSER").as_deref() == Ok("1") || !url.starts_with(&format!("{base}/")) {
        return;
    }
    let mut c = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    if let Err(e) = c.spawn() {
        eprintln!("[kopanod] could not open the browser: {e}");
    }
}

/// Why this computer is signed out, when kopanod did it (unlinked on the website, suspended).
pub fn signed_out_message(home: &Path) -> Option<String> {
    std::fs::read(home.join("signed-out.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["message"].as_str().map(String::from))
}

/// Check a certificate ThabangVision handed this device and keep it. `issuers`: the keys the
/// site publishes. Refused: another device's certificate, a bad signature, an unknown signer,
/// an expired one.
pub fn keep_cert(home: &Path, key: &DeviceKey, cert: &DeviceCert, issuers: &[String]) -> Result<()> {
    if cert.body.device != key.public_hex() {
        bail!("ThabangVision sent a certificate for another computer");
    }
    cert.check(issuers, now_ms()).context("ThabangVision sent a certificate this computer cannot trust")?;
    kopano_core::store::write_atomic(&home.join("issuers.json"), &serde_json::to_vec(issuers)?)?;
    kopano_core::store::write_atomic(&home.join("cert.json"), &serde_json::to_vec_pretty(cert)?)?;
    let _ = std::fs::remove_file(home.join("signed-out.json"));
    Ok(())
}

impl SignIn {
    pub fn new(home: &Path) -> Arc<Self> {
        Arc::new(Self { home: home.into(), flow: Mutex::new(Flow::Idle), task: Mutex::new(None) })
    }

    pub fn flow(&self) -> Flow {
        self.flow.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, f: Flow) {
        *self.flow.lock().unwrap_or_else(|e| e.into_inner()) = f;
    }

    /// The fields `hello` carries while not signed in.
    pub fn fields(&self) -> Value {
        match self.flow() {
            Flow::Waiting { code, link, until_ms } => json!({"signing_in": {"code": code, "link": link, "until_ms": until_ms}, "message": null}),
            Flow::Failed(m) => json!({"signing_in": null, "message": m}),
            Flow::Idle => json!({"signing_in": null, "message": signed_out_message(&self.home)}),
        }
    }

    /// Start signing in (or hand back the code already waiting): `{code, link, until_ms}`.
    pub async fn start(self: &Arc<Self>) -> Result<Value> {
        if let Flow::Waiting { code, link, until_ms } = self.flow() {
            if until_ms > now_ms() + 30_000 {
                open_browser(&link, &tv::base_url(&self.home)?);
                return Ok(json!({"code": code, "link": link, "until_ms": until_ms}));
            }
        }
        self.cancel();
        let tv = Tv::for_home(&self.home)?;
        let key = DeviceKey::from_bytes(keystore::load(&self.home)?);
        let (name, os) = computer();
        let r = tv.post("/api/kopano/link/start", &json!({"device_key": key.public_hex(), "name": name, "os": os})).await?;
        if !r.ok() {
            eprintln!("[kopanod] sign-in start: {} {}", r.status, r.body);
            let m = if r.status == 404 { "Signing in is not available yet.".to_string() } else { plain_error(&r) };
            self.set(Flow::Failed(m.clone()));
            bail!("{m}");
        }
        let b = &r.body;
        let (code, device_code, link) = (
            b["user_code"].as_str().context("no code from ThabangVision")?.to_string(),
            b["device_code"].as_str().context("no code from ThabangVision")?.to_string(),
            b["verification_uri_complete"].as_str().context("no link from ThabangVision")?.to_string(),
        );
        let until_ms = now_ms() + b["expires_in"].as_u64().unwrap_or(600) * 1000;
        let interval = b["interval"].as_u64().unwrap_or(5).max(5);
        self.set(Flow::Waiting { code: code.clone(), link: link.clone(), until_ms });
        open_browser(&link, &tv.base);
        let me = self.clone();
        let t = tokio::spawn(async move {
            let end = me.poll(&tv, &key, &device_code, interval, until_ms).await;
            match end {
                Ok(()) => me.set(Flow::Idle),
                Err(e) => me.set(Flow::Failed(format!("{e:#}"))),
            }
        });
        *self.task.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
        Ok(json!({"code": code, "link": link, "until_ms": until_ms}))
    }

    /// Ask until the person approved, said no, or the code ran out (1b).
    async fn poll(&self, tv: &Tv, key: &DeviceKey, device_code: &str, mut interval: u64, until_ms: u64) -> Result<()> {
        let signature = tv::sign_hex(key, &tv::token_message(device_code));
        loop {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            if now_ms() > until_ms {
                bail!("The sign-in code ran out. Press Sign in again.");
            }
            let r = match tv.post("/api/kopano/link/token", &json!({"device_code": device_code, "signature": signature})).await {
                Ok(r) => r,
                Err(_) => continue, // offline for a moment: keep asking until the code runs out
            };
            match r.word() {
                "approved" => {
                    let cert: DeviceCert = serde_json::from_value(r.body["cert"].clone()).context("ThabangVision sent a certificate this computer cannot read")?;
                    let issuers = tv.issuer_keys().await?;
                    keep_cert(&self.home, key, &cert, &issuers)?;
                    eprintln!("[kopanod] signed in as {}", cert.body.name);
                    return Ok(());
                }
                "pending" | "authorization_pending" => {}
                "slow_down" => interval += 5,
                "denied" | "access_denied" => bail!("Sign-in was not approved in the browser."),
                "expired" | "expired_token" | "invalid" | "invalid_grant" => bail!("The sign-in code ran out. Press Sign in again."),
                "bad_signature" | "invalid_signature" => bail!("Signing in did not work. Press Sign in again."),
                _ if r.status == 429 => interval += 5,
                _ => eprintln!("[kopanod] sign-in poll: {} {}", r.status, r.body),
            }
        }
    }

    pub fn cancel(&self) {
        if let Some(t) = self.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            t.abort();
        }
        self.set(Flow::Idle);
    }
}

/// Forget the certificate and go back to the unlinked service. `message`: why, when it was not
/// the person's own choice (shown by the app's sign-in button).
pub fn sign_out(node: &Node, message: Option<&str>) -> Result<()> {
    let home = &node.device.home;
    match message {
        Some(m) => kopano_core::store::write_atomic(&home.join("signed-out.json"), &serde_json::to_vec(&json!({"message": m}))?)?,
        None => {
            let _ = std::fs::remove_file(home.join("signed-out.json"));
        }
    }
    if home.join("cert.json").exists() {
        std::fs::remove_file(home.join("cert.json"))?;
    }
    node.emit(json!({"event": "account", "signed_in": false, "message": message}));
    node.stop.notify_one();
    Ok(())
}

/// One renewal. `Ok(false)`: ThabangVision refused for good (unlinked, suspended) and this
/// computer is now signed out.
pub async fn renew_once(node: &Node, tv: &Tv) -> Result<bool> {
    let r = tv.renew(&node.device.key).await?;
    if r.ok() {
        let cert: DeviceCert = serde_json::from_value(r.body["cert"].clone()).context("renewal: unreadable certificate")?;
        let issuers = match tv.issuer_keys().await {
            Ok(k) if !k.is_empty() => k,
            _ => serde_json::from_slice(&std::fs::read(node.device.home.join("issuers.json")).unwrap_or_default()).unwrap_or_default(),
        };
        keep_cert(&node.device.home, &node.device.key, &cert, &issuers)?;
        node.device.cert.set(cert);
        let until = node.device.cert.body.expires_ms;
        node.emit(json!({"event": "account", "signed_in": true, "renewed": true, "signed_in_until_ms": until}));
        return Ok(true);
    }
    if r.status == 403 {
        sign_out(node, Some(&plain_error(&r)))?;
        return Ok(false);
    }
    bail!("renewal: {} {}", r.status, r.body)
}

/// Keep this computer's certificate fresh for as long as the service runs.
pub async fn keep_fresh(node: Arc<Node>) {
    // only a certificate ThabangVision issued is renewed there (lab certificates are not)
    let issuers: Vec<String> = serde_json::from_slice(&std::fs::read(node.device.home.join("issuers.json")).unwrap_or_default()).unwrap_or_default();
    if !issuers.contains(&node.device.cert.signer) {
        return;
    }
    let tv = match Tv::for_home(&node.device.home) {
        Ok(t) => t,
        Err(e) => return eprintln!("[kopanod] renewal off: {e:#}"),
    };
    loop {
        let left = node.device.cert.body.expires_ms.saturating_sub(now_ms());
        let before = renew_before_ms();
        if left > before {
            tokio::time::sleep(Duration::from_millis((left - before).min(6 * 3600 * 1000))).await;
            continue;
        }
        match renew_once(&node, &tv).await {
            Ok(true) => eprintln!("[kopanod] certificate renewed until {}", node.device.cert.body.expires_ms),
            Ok(false) => return,
            Err(e) => {
                eprintln!("[kopanod] {e:#}; trying again later");
                // an expired certificate keeps this computer off its projects: try sooner
                let wait = if left == 0 { 60 } else { 15 * 60 };
                tokio::time::sleep(Duration::from_secs(wait)).await;
            }
        }
        if std::env::var("KOPANO_RENEW_BEFORE_MS").is_ok() {
            // tests force renewal: once is enough, not a loop against the rate limit
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kopano_core::identity::Provider;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kopano-signin-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn only_a_fresh_certificate_for_this_device_from_a_published_key_is_kept() {
        let home = temp("keep");
        let (tv, other) = (Provider::generate(), Provider::generate());
        let key = DeviceKey::generate();
        let issuers = vec![tv.public_hex()];
        // another computer's certificate
        let theirs = tv.issue("acct", "Thabang", &DeviceKey::generate(), "PC", DAY_MS).unwrap();
        assert!(keep_cert(&home, &key, &theirs, &issuers).is_err());
        // signed by a key the site does not publish
        let forged = other.issue("acct", "Thabang", &key, "PC", DAY_MS).unwrap();
        assert!(keep_cert(&home, &key, &forged, &issuers).is_err());
        // expired on arrival
        let mut old = tv.issue("acct", "Thabang", &key, "PC", DAY_MS).unwrap();
        old.body.expires_ms = now_ms() - 1;
        let old = kopano_core::identity::Signed::new(old.body, &kopano_core::identity::DeviceKey::from_bytes(tv.to_bytes()).signing().clone()).unwrap();
        assert!(keep_cert(&home, &key, &old, &issuers).is_err());
        assert!(!home.join("cert.json").exists(), "nothing refused was written");
        // the real one, and a sign-out note from before is cleared
        std::fs::write(home.join("signed-out.json"), br#"{"message":"x"}"#).unwrap();
        let good = tv.issue("acct", "Thabang", &key, "PC", 30 * DAY_MS).unwrap();
        keep_cert(&home, &key, &good, &issuers).unwrap();
        assert!(home.join("cert.json").exists());
        assert_eq!(signed_out_message(&home), None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_waiting_code_shows_in_hello_and_a_cancel_clears_it() {
        let home = temp("flow");
        let s = SignIn::new(&home);
        assert_eq!(s.fields()["signing_in"], Value::Null);
        s.set(Flow::Waiting { code: "BCDF-1234".into(), link: "http://localhost:3100/link?code=BCDF-1234".into(), until_ms: now_ms() + 600_000 });
        assert_eq!(s.fields()["signing_in"]["code"], "BCDF-1234");
        s.cancel();
        assert_eq!(s.flow(), Flow::Idle);
        std::fs::write(home.join("signed-out.json"), br#"{"message":"This computer was unlinked from ThabangVision. Sign in again."}"#).unwrap();
        assert!(s.fields()["message"].as_str().unwrap().contains("unlinked"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_computer_label_is_never_empty() {
        let (name, os) = computer();
        assert!(!name.is_empty() && !os.is_empty());
    }
}
