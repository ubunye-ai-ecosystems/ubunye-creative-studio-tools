//! ThabangVision, as kopanod sees it (09 sections 3 and 4). The contract is
//! `docs/kopano/desktop-api.md` in the ThabangVision repo; every route and message below follows
//! it word for word.
//!
//! kopanod talks to ThabangVision only to sign in, renew its certificate and use the project
//! mirror (find and invite people, "Shared with me"). Who may edit a project is decided by the
//! project's signed members log (approve.rs), never by an answer from here.
//!
//! Base URL: `KOPANO_TV_URL`, else `<home>/server.json` `{"url": ...}`, else the live site
//! (owner approved 2026-10-31). Tests and dev runs point at `http://localhost:3100` with either.

use anyhow::{anyhow, bail, Context, Result};
use kopano_core::identity::{now_ms, DeviceKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

/// The live ThabangVision site (desktop-api.md, "Base URL").
pub const DEFAULT_URL: &str = "https://www.thabangvision.com";

pub fn base_url(home: &Path) -> Result<String> {
    let url = std::env::var("KOPANO_TV_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| std::fs::read(home.join("server.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["url"].as_str().map(String::from)))
        .unwrap_or_else(|| DEFAULT_URL.to_string());
    Ok(url.trim_end_matches('/').to_string())
}

pub fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// What the device signs to collect its certificate (desktop-api.md 1b).
pub fn token_message(device_code: &str) -> String {
    format!("kopano-link-token:v1:{device_code}")
}

/// What the device signs to renew (1c).
pub fn renew_message(device: &str, ts: u64) -> String {
    format!("kopano-cert-renew:v1:{device}:{ts}")
}

/// What the device signs for a project route (section 3).
pub fn api_message(method: &str, path_and_query: &str, device: &str, ts: u64, body: &[u8]) -> String {
    format!("kopano-api:v1:{}:{path_and_query}:{device}:{ts}:{}", method.to_uppercase(), sha256_hex(body))
}

pub fn sign_hex(key: &DeviceKey, msg: &str) -> String {
    use ed25519_dalek::Signer;
    hex::encode(key.signing().sign(msg.as_bytes()).to_bytes())
}

/// One answer: the HTTP status and the JSON body (`null` when the body was not JSON).
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    /// The machine word ThabangVision answered with (`status`, else `error`).
    pub fn word(&self) -> &str {
        self.body.get("status").and_then(Value::as_str).or_else(|| self.body.get("error").and_then(Value::as_str)).unwrap_or("")
    }
}

/// A sentence for the person in the app for an answer that is not a success. Never names the
/// protocol; ThabangVision's own sentences ("Project not found.") are passed on as they are.
pub fn plain_error(r: &Reply) -> String {
    let said = |k: &str| r.body.get(k).and_then(Value::as_str).filter(|s| s.contains(' ')).map(String::from);
    match (r.status, r.word()) {
        (401, "stale") | (400, "stale") => "This computer's clock is wrong. Set the time automatically and try again.".into(),
        (403, "revoked") => "This computer was unlinked from ThabangVision. Sign in again.".into(),
        (403, "suspended") => "This ThabangVision account is suspended.".into(),
        (401, _) => "This computer is not signed in to ThabangVision. Sign in again.".into(),
        (429, _) => "Too many tries. Wait a few minutes and try again.".into(),
        (s, _) if s >= 500 => "ThabangVision is not answering right now. Try again in a few minutes.".into(),
        (404, "Not found.") => "Sharing is not available yet.".into(),
        _ => said("error").or_else(|| said("message")).unwrap_or_else(|| "ThabangVision could not do that. Try again.".into()),
    }
}

pub const UNREACHABLE: &str = "ThabangVision cannot be reached. Check the internet connection and try again.";

pub struct Tv {
    pub base: String,
    http: reqwest::Client,
}

impl Tv {
    pub fn new(base: &str) -> Result<Self> {
        // reqwest is built without a default crypto provider (iroh's choice): give it ring
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).connect_timeout(Duration::from_secs(10)).build().context("HTTP client")?;
        Ok(Self { base: base.trim_end_matches('/').to_string(), http })
    }

    pub fn for_home(home: &Path) -> Result<Self> {
        Self::new(&base_url(home)?)
    }

    async fn send(&self, rb: reqwest::RequestBuilder) -> Result<Reply> {
        let res = rb.send().await.map_err(|e| {
            eprintln!("[kopanod] ThabangVision: {e}");
            anyhow!(UNREACHABLE)
        })?;
        let status = res.status().as_u16();
        let text = res.text().await.map_err(|_| anyhow!(UNREACHABLE))?;
        Ok(Reply { status, body: serde_json::from_str(&text).unwrap_or(Value::Null) })
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Reply> {
        self.send(self.http.post(format!("{}{path}", self.base)).header("content-type", "application/json").body(body.to_string())).await
    }

    pub async fn get(&self, path: &str) -> Result<Reply> {
        self.send(self.http.get(format!("{}{path}", self.base))).await
    }

    /// A project route, signed by this computer's key (section 3).
    pub async fn signed(&self, key: &DeviceKey, method: &str, path_and_query: &str, body: Option<&Value>) -> Result<Reply> {
        let bytes = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
        let ts = now_ms();
        let device = key.public_hex();
        let sig = sign_hex(key, &api_message(method, path_and_query, &device, ts, &bytes));
        let url = format!("{}{path_and_query}", self.base);
        let rb = match method {
            "GET" => self.http.get(url),
            "POST" => self.http.post(url).header("content-type", "application/json").body(bytes),
            m => bail!("method {m}"),
        };
        self.send(rb.header("x-kopano-device", device).header("x-kopano-timestamp", ts.to_string()).header("x-kopano-signature", sig)).await
    }

    /// A signed call that must succeed; anything else becomes a plain sentence.
    pub async fn call(&self, key: &DeviceKey, method: &str, path_and_query: &str, body: Option<&Value>) -> Result<Value> {
        let r = self.signed(key, method, path_and_query, body).await?;
        if !r.ok() {
            eprintln!("[kopanod] ThabangVision {method} {path_and_query}: {} {}", r.status, r.body);
            bail!("{}", plain_error(&r));
        }
        Ok(r.body)
    }

    /// The issuer keys peers trust (1d): the `public_key` of every active key.
    pub async fn issuer_keys(&self) -> Result<Vec<String>> {
        let r = self.get("/.well-known/kopano-keys.json").await?;
        if !r.ok() {
            bail!("{}", plain_error(&r));
        }
        Ok(r.body["keys"].as_array().into_iter().flatten().filter(|k| k["status"] != "retired").filter_map(|k| k["public_key"].as_str().map(String::from)).collect())
    }

    pub async fn renew(&self, key: &DeviceKey) -> Result<Reply> {
        let ts = now_ms();
        let device = key.public_hex();
        self.post("/api/kopano/cert/renew", &json!({"device_key": device, "timestamp": ts, "signature": sign_hex(key, &renew_message(&device, ts))})).await
    }
}

/// The path of a GET with one query value (ids from ThabangVision are uuids: nothing to escape,
/// but anything else is refused rather than sent).
pub fn with_query(path: &str, name: &str, value: &str) -> Result<String> {
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        bail!("that is not a ThabangVision id");
    }
    Ok(format!("{path}?{name}={value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vector ThabangVision's tests also check (kopano-core tests/vectors).
    fn vector() -> Value {
        serde_json::from_str(include_str!("../../kopano-core/tests/vectors/thabangvision-device-cert.json")).unwrap()
    }

    fn device(v: &Value) -> DeviceKey {
        DeviceKey::from_bytes(hex::decode(v["device_seed"].as_str().unwrap()).unwrap().try_into().unwrap())
    }

    #[test]
    fn the_link_and_renew_signatures_are_the_ones_thabangvision_checks() {
        let v = vector();
        let k = device(&v);
        let code = v["token"]["device_code"].as_str().unwrap();
        assert_eq!(sign_hex(&k, &token_message(code)), v["token"]["signature"].as_str().unwrap());
        let ts = v["renew"]["timestamp"].as_u64().unwrap();
        assert_eq!(renew_message(&k.public_hex(), ts), v["renew"]["message"].as_str().unwrap());
        assert_eq!(sign_hex(&k, &renew_message(&k.public_hex(), ts)), v["renew"]["signature"].as_str().unwrap());
    }

    #[test]
    fn a_signed_route_covers_method_path_device_time_and_body() {
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(api_message("get", "/api/kopano/projects", "ab", 7, b""), format!("kopano-api:v1:GET:/api/kopano/projects:ab:7:{empty}"));
        let m = api_message("POST", "/api/kopano/projects/invite", "ab", 7, br#"{"a":1}"#);
        assert!(m.ends_with(&sha256_hex(br#"{"a":1}"#)) && !m.ends_with(empty));
    }

    #[test]
    fn answers_become_plain_sentences_without_protocol_words() {
        let r = |status, body: Value| plain_error(&Reply { status, body });
        assert!(r(403, json!({"status": "revoked", "error": "revoked"})).contains("unlinked"));
        assert!(r(401, json!({"error": "stale"})).contains("clock"));
        assert_eq!(r(404, json!({"error": "Project not found."})), "Project not found.");
        assert_eq!(r(404, json!({"error": "Not found."})), "Sharing is not available yet.");
        assert!(r(503, json!({"error": "unavailable"})).contains("not answering"));
        assert!(r(400, json!({"error": "invalid_request"})).contains("could not"));
        for s in [r(429, Value::Null), r(500, Value::Null), r(401, Value::Null)] {
            assert!(!s.to_lowercase().contains("kopano"), "{s}");
        }
    }

    #[test]
    fn the_live_site_unless_told_otherwise() {
        let d = std::env::temp_dir().join(format!("kopano-tv-url-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        if std::env::var_os("KOPANO_TV_URL").is_none() {
            assert_eq!(base_url(&d).unwrap(), "https://www.thabangvision.com");
            std::fs::write(d.join("server.json"), br#"{"url": "http://localhost:3100/"}"#).unwrap();
            assert_eq!(base_url(&d).unwrap(), "http://localhost:3100");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ids_in_a_query_are_checked_not_escaped() {
        assert!(with_query("/x", "project_id", "6f1c2a4e-1d2b-4c3a-9e8f-0123456789ab").is_ok());
        assert!(with_query("/x", "project_id", "a&b=c").is_err());
        assert!(with_query("/x", "project_id", "").is_err());
    }
}
