//! Device keys at rest (security pass, 2026-10-18). A device key used to sit in `device.key` as 32
//! plain bytes: anyone who copied the folder could act as that device. `protect` moves it into the
//! operating system's key store (Windows Credential Manager, macOS Keychain), which ties it to the
//! signed-in user; `device.key` then only names the entry. Linux has no store we can rely on
//! offline, so there the key stays in the file and the file is made readable by its owner only.

use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;

const SERVICE: &str = "kopano";
const MARK: &[u8] = b"keystore:";

pub fn load(dir: &Path) -> Result<[u8; 32]> {
    let raw = std::fs::read(dir.join("device.key")).context("no device key")?;
    if let Some(name) = raw.strip_prefix(MARK) {
        let name = std::str::from_utf8(name)?.trim();
        let secret = entry(name)?.get_secret().map_err(|e| anyhow!("device key {name} not in this user's key store ({e}); was the folder copied from another user or machine?"))?;
        return secret.try_into().map_err(|_| anyhow!("bad device key in key store"));
    }
    raw.try_into().map_err(|_| anyhow!("bad device key"))
}

/// Move a plain key into the OS key store. Returns the entry name.
pub fn protect(dir: &Path) -> Result<String> {
    let path = dir.join("device.key");
    let raw = std::fs::read(&path)?;
    if raw.starts_with(MARK) {
        return Ok(String::from_utf8(raw[MARK.len()..].to_vec())?);
    }
    let key: [u8; 32] = raw.try_into().map_err(|_| anyhow!("bad device key"))?;
    if !cfg!(any(windows, target_os = "macos")) {
        restrict(&path)?;
        bail!("no OS key store on this system; key left in device.key, readable by its owner only");
    }
    let name = format!("device-{}", hex::encode(&blake3::hash(&key).as_bytes()[..8]));
    entry(&name)?.set_secret(&key).map_err(|e| anyhow!("key store refused the key: {e}"))?;
    // read it back through a fresh handle before the plain copy goes
    let back = entry(&name)?.get_secret().map_err(|e| anyhow!("key store did not keep the key: {e}"))?;
    if back != key {
        bail!("key store returned a different key; plain key kept");
    }
    let tmp = dir.join("device.key.tmp");
    std::fs::write(&tmp, [MARK, name.as_bytes()].concat())?;
    std::fs::rename(&tmp, &path)?;
    Ok(name)
}

fn entry(name: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, name).map_err(|e| anyhow!("key store unavailable: {e}"))
}

#[cfg(unix)]
fn restrict(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}
#[cfg(not(unix))]
fn restrict(_: &Path) -> Result<()> {
    Ok(())
}
