//! The local API before this computer is signed in (slice S4, installers).
//!
//! A fresh install has a device key (`kopanod init`) but no `cert.json` yet: that comes from
//! "Sign in with ThabangVision" (slice S2). Until then kopanod still answers the apps, so their
//! window can say "not signed in" and show the sign-in button instead of "not running". It
//! never goes online in this state. Every 2 s it looks for `cert.json`; when it appears, open
//! connections are closed (the apps reconnect by themselves) and the real service starts on the
//! same port without a gap.
//!
//! Signing in starts here (signin.rs): `sign-in` answers with the code and opens the browser,
//! `hello` shows the waiting code (or why the last try ended), `sign-in-cancel` stops it. The
//! certificate it collects is the `cert.json` this loop waits for.

use crate::signin::SignIn;
use anyhow::Result;
use kopano_core::identity::DeviceKey;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// What `hello` says while not signed in: who this device is, the sign-in under way (its code
/// and link) or why the last one ended, and nothing else.
pub fn hello(device: &str, signin: &Value) -> Value {
    json!({"device": device, "name": "", "account": "", "signed_in": false, "signed_in_until_ms": null, "projects": [],
        "signing_in": signin["signing_in"], "message": signin["message"]})
}

/// The reply to one request while not signed in.
pub async fn reply(device: &str, signin: &Arc<SignIn>, req: &Value) -> Value {
    match req.get("op").and_then(Value::as_str) {
        Some("hello") => json!({"ok": true, "result": hello(device, &signin.fields())}),
        Some("sign-in") => match signin.start().await {
            Ok(v) => json!({"ok": true, "result": v}),
            Err(e) => json!({"ok": false, "error": format!("{e:#}")}),
        },
        Some("sign-in-cancel") => {
            signin.cancel();
            json!({"ok": true, "result": "not signing in"})
        }
        Some("sign-out") => json!({"ok": true, "result": "signed out"}),
        _ => json!({"ok": false, "error": "This computer is not signed in to ThabangVision yet."}),
    }
}

/// Serve on `l` until `<home>/cert.json` exists.
pub async fn serve_until_signed_in(home: &Path, l: &TcpListener, token: String) -> Result<()> {
    let device = DeviceKey::from_bytes(crate::keystore::load(home)?).public_hex();
    let signin = SignIn::new(home);
    let (close_tx, close_rx) = tokio::sync::watch::channel(false);
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    eprintln!("[kopanod] not signed in yet; answering apps on the local port only");
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if home.join("cert.json").exists() {
                    let _ = close_tx.send(true);
                    signin.cancel();
                    eprintln!("[kopanod] signed in; starting");
                    return Ok(());
                }
            }
            c = l.accept() => {
                let Ok((c, _)) = c else { continue };
                let (device, token, mut closed, signin) = (device.clone(), token.clone(), close_rx.clone(), signin.clone());
                tokio::spawn(async move {
                    let (r, mut w) = c.into_split();
                    let mut lines = BufReader::new(r).lines();
                    loop {
                        let line = tokio::select! {
                            l = lines.next_line() => match l { Ok(Some(l)) => l, _ => return },
                            _ = closed.changed() => return,
                        };
                        let req: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                        if req.get("token").and_then(Value::as_str) != Some(token.as_str()) {
                            let _ = w.write_all(b"{\"ok\":false,\"error\":\"bad token\"}\n").await;
                            return;
                        }
                        if req["op"] == "subscribe" {
                            // no events until signed in; hang up when the real service takes over
                            let _ = w.write_all(b"{\"ok\":true,\"result\":\"subscribed\"}\n").await;
                            let _ = closed.changed().await;
                            return;
                        }
                        let out = reply(&device, &signin, &req).await;
                        if w.write_all(format!("{out}\n").as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hello_says_not_signed_in_and_other_ops_refuse() {
        let s = SignIn::new(&std::env::temp_dir().join(format!("kopano-unlinked-{}", std::process::id())));
        let ok = reply("ab", &s, &json!({"op": "hello"})).await;
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["result"]["signed_in"], false);
        assert_eq!(ok["result"]["device"], "ab");
        assert_eq!(ok["result"]["projects"], json!([]));
        assert_eq!(ok["result"]["signing_in"], Value::Null);
        for op in [json!({"op": "create-project", "name": "x"}), json!({"op": "invite", "project": "x", "email": "a@b.c"}), json!({"op": "shared-projects"})] {
            let no = reply("ab", &s, &op).await;
            assert_eq!(no["ok"], false);
            assert!(!no["error"].as_str().unwrap().contains("Kopano"));
        }
        assert_eq!(reply("ab", &s, &json!({"op": "sign-in-cancel"})).await["ok"], true);
    }
}
