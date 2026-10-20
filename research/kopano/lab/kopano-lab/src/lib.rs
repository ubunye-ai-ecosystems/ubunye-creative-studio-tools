//! Lab helpers: a toy image-editing app with a Kopano connector. It stands in for a real editor
//! (Light, Photo, Film) so the core can be tested without any of them, which is the point: the
//! core must not care which app is on the other side.

pub mod film;
pub mod light;

use anyhow::{bail, Context, Result};
use kopano_core::conformance::Harness;
use kopano_core::connector::{Connector, ConnectorInfo, KindInfo, CONTRACT_VERSION};
use kopano_core::project::{AssetRef, Item};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// A pretend photo editor: each photo has develop settings, a rating, and references its
/// original file by BLAKE3 hash. Edits never touch the original.
#[derive(Clone)]
pub struct ToyEditor {
    pub photos: BTreeMap<String, Item>,
}

impl ToyEditor {
    /// `originals`: (stable key, BLAKE3 hex, size) of each original file.
    pub fn new(originals: &[(String, String, u64)]) -> Self {
        let mut photos = BTreeMap::new();
        for (key, hash, size) in originals {
            photos.insert(
                key.clone(),
                Item {
                    kind: "lab.photo".into(),
                    lane: "develop".into(),
                    schema: 1,
                    state: json!({
                        "develop": {"exposure": 0.0, "contrast": 0.0, "temperature": 5500.0, "masks": []},
                        "rating": 0,
                        "flag": "none"
                    }),
                    assets: vec![AssetRef { hash: hash.clone(), size: *size, role: "original".into() }],
                },
            );
        }
        Self { photos }
    }

    pub fn demo(n: usize) -> Self {
        let originals: Vec<(String, String, u64)> = (0..n)
            .map(|i| (format!("photo-{i:04}"), blake3::hash(format!("original {i}").as_bytes()).to_hex().to_string(), 50_000_000))
            .collect();
        Self::new(&originals)
    }

    fn field_mut<'a>(item: &'a mut Item, path: &str) -> Result<&'a mut Value> {
        let ptr = format!("/{}", path.replace('.', "/"));
        item.state.pointer_mut(&ptr).with_context(|| format!("no setting {path}"))
    }
}

impl Connector for ToyEditor {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            app: "lab.toy-editor".into(),
            contract: CONTRACT_VERSION,
            kinds: vec![KindInfo { kind: "lab.photo".into(), lane: "develop".into(), schema: 1, mergeable: true }],
        }
    }
    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
        Ok(self.photos.clone())
    }
    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
        match item {
            Some(i) => {
                if i.kind != "lab.photo" {
                    bail!("toy editor cannot hold {}", i.kind);
                }
                self.photos.insert(key.into(), i.clone());
            }
            None => {
                self.photos.remove(key);
            }
        }
        Ok(())
    }
}

impl Harness for ToyEditor {
    fn fresh(&self) -> Self {
        Self::demo(self.photos.len().max(3))
    }
    fn set_number(&mut self, key: &str, path: &str, value: f64) -> Result<()> {
        let item = self.photos.get_mut(key).with_context(|| format!("no photo {key}"))?;
        *Self::field_mut(item, path)? = json!(value);
        Ok(())
    }
    fn get_number(&mut self, key: &str, path: &str) -> Result<f64> {
        let item = self.photos.get_mut(key).with_context(|| format!("no photo {key}"))?;
        Self::field_mut(item, path)?.as_f64().context("not a number")
    }
    fn sample(&self) -> (String, String, String) {
        ("photo-0001".into(), "develop.exposure".into(), "develop.contrast".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toy_editor_passes_conformance() {
        let h = ToyEditor::demo(5);
        let mut failed = Vec::new();
        for o in kopano_core::conformance::run(&h) {
            match &o.result {
                Ok(()) => println!("PASS  {}", o.name),
                Err(e) => {
                    println!("FAIL  {}: {e:#}", o.name);
                    failed.push(o.name);
                }
            }
        }
        assert!(failed.is_empty(), "failed: {failed:?}");
    }

    /// The same kit against the real Ubunye Light engine (headless, in-memory demo library).
    /// Runs only when KOPANO_LIGHT_CLI points at lightcraft-cli.
    #[test]
    fn ubunye_light_passes_conformance() {
        let Ok(cli) = std::env::var("KOPANO_LIGHT_CLI") else {
            println!("skipped: set KOPANO_LIGHT_CLI");
            return;
        };
        let h = crate::light::LightConnector::headless(&cli).unwrap();
        let mut failed = Vec::new();
        for o in kopano_core::conformance::run(&h) {
            match &o.result {
                Ok(()) => println!("PASS  {}", o.name),
                Err(e) => {
                    println!("FAIL  {}: {e:#}", o.name);
                    failed.push(o.name);
                }
            }
        }
        assert!(failed.is_empty(), "failed: {failed:?}");
    }

    fn conformance_report<H: kopano_core::conformance::Harness>(h: &H) -> Vec<&'static str> {
        let mut failed = Vec::new();
        for o in kopano_core::conformance::run(h) {
            match &o.result {
                Ok(()) => println!("PASS  {}", o.name),
                Err(e) => {
                    println!("FAIL  {}: {e:#}", o.name);
                    failed.push(o.name);
                }
            }
        }
        failed
    }

    /// The kit against the real Ubunye Film engine (headless `filmcraft-cli mcp`), on Film's
    /// demo project or on `KOPANO_FILM_PROJECT`. Runs only when KOPANO_FILM_CLI points at
    /// filmcraft-cli built from Film's feature/kopano-stable-ids branch.
    #[test]
    fn ubunye_film_passes_conformance() {
        let Ok(cli) = std::env::var("KOPANO_FILM_CLI") else {
            println!("skipped: set KOPANO_FILM_CLI");
            return;
        };
        let project = std::env::var("KOPANO_FILM_PROJECT").ok();
        let h = crate::film::FilmConnector::headless(&cli, project.as_deref()).unwrap();
        let failed = conformance_report(&h);
        assert!(failed.is_empty(), "failed: {failed:?}");
    }

    /// The same kit against two running Film windows (`filmcraft --control PORT`), each fresh
    /// instance reopening KOPANO_FILM_PROJECT. Runs only when KOPANO_FILM_PORTS is set
    /// ("18401,18402").
    #[test]
    fn ubunye_film_windows_pass_conformance() {
        let (Ok(ports), Ok(project)) = (std::env::var("KOPANO_FILM_PORTS"), std::env::var("KOPANO_FILM_PROJECT")) else {
            println!("skipped: set KOPANO_FILM_PORTS and KOPANO_FILM_PROJECT");
            return;
        };
        let ports: Vec<u16> = ports.split(',').map(|p| p.trim().parse().unwrap()).collect();
        let fresh = crate::film::Fresh::Windows { ports: ports.clone(), project: project.clone() };
        let mut h = crate::film::FilmConnector::live(ports[0], fresh).unwrap();
        h.open(&project).unwrap();
        let failed = conformance_report(&h);
        assert!(failed.is_empty(), "failed: {failed:?}");
    }
}
