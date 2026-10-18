//! E23: save cost at scale, full snapshot versus change feed. Run with E23_N=100000,300000,...
use anyhow::Result;
use kopano_core::connector::Connector;
use kopano_core::identity::{DeviceKey, Provider};
use kopano_core::project::Project;
use kopano_core::repo::Repo;
use kopano_core::store::MemStore;
use kopano_lab::ToyEditor;
use serde_json::json;
use std::time::Instant;

#[test]
fn e23_scale() -> Result<()> {
    let ns: Vec<usize> = std::env::var("E23_N").unwrap_or("10000,100000".into()).split(',').map(|x| x.parse().unwrap()).collect();
    for n in ns {
        let store = MemStore::default();
        let provider = Provider::generate();
        let key = DeviceKey::generate();
        let cert = provider.issue("a", "A", &key, "pc", 86_400_000)?;
        let p = Project::create(&store, "scale", vec![provider.public_hex()], &cert, &key, Default::default())?;
        let mut app = ToyEditor::demo(n);
        let mut repo = Repo { store: &store, project: Project::load(&store, &p.id)?, head: None, cert: cert.clone(), key: DeviceKey::from_bytes(key.to_bytes()) };
        let t = Instant::now();
        repo.save(&mut app, "import")?;
        let import_s = t.elapsed().as_secs_f64();
        let keys: Vec<String> = app.photos.keys().step_by(n / 10).cloned().collect();
        // full snapshot saves
        let t = Instant::now();
        for (i, k) in keys.iter().enumerate() {
            app.photos.get_mut(k).unwrap().state["develop"]["exposure"] = json!(i as f64 * 0.1);
            repo.save(&mut app, "edit")?;
        }
        let full_ms = t.elapsed().as_secs_f64() * 1000.0 / keys.len() as f64;
        // change feed saves
        let info = app.info();
        let t = Instant::now();
        for (i, k) in keys.iter().enumerate() {
            let it = app.photos.get_mut(k).unwrap();
            it.state["develop"]["contrast"] = json!(i as f64);
            let item = it.clone();
            repo.save_changes(&info, vec![(k.clone(), Some(item))], "edit")?;
        }
        let feed_ms = t.elapsed().as_secs_f64() * 1000.0 / keys.len() as f64;
        println!("{}", json!({"items": n, "import_s": (import_s * 10.0).round() / 10.0, "save_full_snapshot_ms": (full_ms * 10.0).round() / 10.0, "save_change_feed_ms": (feed_ms * 100.0).round() / 100.0}));
    }
    Ok(())
}
