//! Lab helpers: a toy image-editing app with a Kopano connector. It stands in for a real editor
//! (Light, Photo, Film) so the core can be tested without any of them, which is the point: the
//! core must not care which app is on the other side.

pub mod light;
pub mod photo;

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

    /// The same kit against the real Ubunye Photo engine (headless `photocraft-cli serve`, an
    /// in-memory demo document). Runs only when KOPANO_PHOTO_CLI points at photocraft-cli.
    #[test]
    fn ubunye_photo_passes_conformance() {
        let Ok(cli) = std::env::var("KOPANO_PHOTO_CLI") else {
            println!("skipped: set KOPANO_PHOTO_CLI");
            return;
        };
        let media = std::env::temp_dir().join(format!("kopano-photo-conformance-{}", std::process::id()));
        let h = crate::photo::PhotoConnector::headless_demo(&cli, &media).unwrap();
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
        drop(h);
        let _ = std::fs::remove_dir_all(&media);
        assert!(failed.is_empty(), "failed: {failed:?}");
    }
}

#[cfg(test)]
mod photo_tiles {
    use kopano_core::connector::Connector;
    use kopano_core::identity::{DeviceKey, Provider};
    use kopano_core::project::Project;
    use kopano_core::repo::{copy_version_closure, Repo};
    use kopano_core::store::{MemStore, ObjectStore};
    use serde_json::json;

    /// A brush stroke on the real Photo engine changes exactly the tile items it touched, and an
    /// adjustment change moves no pixels at all. Runs only when KOPANO_PHOTO_CLI is set.
    #[test]
    fn photo_brush_stroke_moves_only_touched_tiles() {
        let Ok(cli) = std::env::var("KOPANO_PHOTO_CLI") else {
            println!("skipped: set KOPANO_PHOTO_CLI");
            return;
        };
        let media = std::env::temp_dir().join(format!("kopano-photo-tiles-{}", std::process::id()));
        let mut a = crate::photo::PhotoConnector::headless_demo(&cli, &media).unwrap();
        let provider = Provider::generate();
        let (sa, sb) = (MemStore::default(), MemStore::default());
        let key = DeviceKey::generate();
        let cert = provider.issue("alice", "Alice", &key, "test", 3_600_000).unwrap();
        let p = Project::create(&sa, "tiles", vec![provider.public_hex()], &cert, &key, Default::default()).unwrap();
        sb.put_verified(&p.id, &sa.get(&p.id).unwrap()).unwrap();
        let mut r = Repo { store: &sa, project: Project::load(&sa, &p.id).unwrap(), head: None, cert, key };
        let s0 = a.snapshot().unwrap();
        let v0 = r.save(&mut a, "start").unwrap().unwrap();
        copy_version_closure(&sa, &sb, &v0).unwrap();
        let changed = |x: &std::collections::BTreeMap<String, kopano_core::project::Item>, y: &std::collections::BTreeMap<String, kopano_core::project::Item>| -> Vec<String> {
            x.keys().chain(y.keys()).collect::<std::collections::BTreeSet<_>>().into_iter().filter(|k| x.get(*k) != y.get(*k)).cloned().collect()
        };
        // One short stroke inside tile (1, 0) of the Retouch layer (tiles are 256 px).
        let retouch = s0.iter().find(|(k, i)| !k.contains('@') && i.state["name"] == "Retouch").map(|(k, _)| k.trim_start_matches("layer/").to_string()).unwrap();
        let id = a.layer_id(&retouch).unwrap();
        a.t.exec("layer.select", json!({"layer": id})).unwrap();
        a.t.exec("paint.stroke", json!({"points": [[300, 60, 1], [360, 90, 1]], "size": 12, "color": "#2060ff"})).unwrap();
        let s1 = a.snapshot().unwrap();
        let keys = changed(&s0, &s1);
        let v1 = r.save(&mut a, "stroke").unwrap().unwrap();
        let (_, bytes) = copy_version_closure(&sa, &sb, &v1).unwrap();
        let media_raw: u64 = keys.iter().filter_map(|k| s1.get(k)).flat_map(|i| &i.assets).map(|x| x.size).sum();
        println!("{}", json!({"edit": "one brush stroke", "changed": keys, "kopano_bytes": bytes, "media_raw_bytes": media_raw}));
        assert_eq!(keys, vec![format!("layer/{retouch}@/content/surface@1_0")], "only the touched tile changes");
        // An adjustment change: one layer item, no media.
        let (adj, field, _) = kopano_core::conformance::Harness::sample(&a);
        kopano_core::conformance::Harness::set_number(&mut a, &adj, &field, 0.8).unwrap();
        let s2 = a.snapshot().unwrap();
        let keys = changed(&s1, &s2);
        let v2 = r.save(&mut a, "adjust").unwrap().unwrap();
        let (_, bytes) = copy_version_closure(&sa, &sb, &v2).unwrap();
        println!("{}", json!({"edit": "exposure slider", "changed": keys, "kopano_bytes": bytes}));
        assert_eq!(keys, vec![adj.clone()]);
        assert!(s2[&adj].assets.is_empty());
        drop(a);
        let _ = std::fs::remove_dir_all(&media);
    }
}
