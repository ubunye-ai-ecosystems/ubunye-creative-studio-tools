//! Content-addressed object store. Every object is named by the BLAKE3 hash of its bytes, so
//! two peers that hold the same bytes agree on the name, and any damage is detectable on read.
//! This store is for Kopano's own small objects (versions, tree nodes, item states). Large media
//! lives in a separate blob store with verified range transfer (see the lab's iroh experiment).

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub [u8; 32]);

impl Id {
    pub fn of(bytes: &[u8]) -> Self {
        Id(*blake3::hash(bytes).as_bytes())
    }
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }
    pub fn short(&self) -> String {
        hex::encode(&self.0[..4])
    }
    pub fn parse(s: &str) -> Result<Self> {
        let b: [u8; 32] = hex::decode(s)
            .ok()
            .and_then(|v| v.try_into().ok())
            .with_context(|| format!("not an object id: {s}"))?;
        Ok(Id(b))
    }
}

impl std::fmt::Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.short())
    }
}

pub trait ObjectStore: Send + Sync {
    fn put(&self, bytes: &[u8]) -> Result<Id>;
    fn get(&self, id: &Id) -> Result<Vec<u8>>;
    fn has(&self, id: &Id) -> bool;
    /// Make every `put` so far durable, as one transaction. Stores that write through need nothing.
    fn flush(&self) -> Result<()> {
        Ok(())
    }
    /// Store bytes received from someone else: refused unless they match the claimed name.
    fn put_verified(&self, id: &Id, bytes: &[u8]) -> Result<()> {
        if Id::of(bytes) != *id {
            bail!("object {} refused: contents do not match its name", id.short());
        }
        self.put(bytes)?;
        Ok(())
    }
}

/// In-memory store, for tests and benchmarks. Counts bytes written so experiments can report
/// exactly how much new data an operation produced.
#[derive(Default)]
pub struct MemStore {
    map: Mutex<HashMap<Id, Vec<u8>>>,
    written: Mutex<u64>,
}

impl MemStore {
    pub fn bytes_written(&self) -> u64 {
        *self.written.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn reset_counter(&self) {
        *self.written.lock().unwrap_or_else(|e| e.into_inner()) = 0;
    }
    pub fn len(&self) -> usize {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ObjectStore for MemStore {
    fn put(&self, bytes: &[u8]) -> Result<Id> {
        let id = Id::of(bytes);
        let mut m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if !m.contains_key(&id) {
            m.insert(id, bytes.to_vec());
            *self.written.lock().unwrap_or_else(|e| e.into_inner()) += bytes.len() as u64;
        }
        Ok(id)
    }
    fn get(&self, id: &Id) -> Result<Vec<u8>> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .with_context(|| format!("missing object {}", id.short()))
    }
    fn has(&self, id: &Id) -> bool {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).contains_key(id)
    }
}

/// Objects as files: `objects/ab/abcdef….` Written to a temp file then renamed, so a crash
/// never leaves a half-written object under a valid name. Reads re-hash and refuse damage.
pub struct FsStore {
    root: PathBuf,
}

impl FsStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("objects")).with_context(|| format!("cannot create {}", root.display()))?;
        Ok(Self { root })
    }
    fn path(&self, id: &Id) -> PathBuf {
        let h = id.hex();
        self.root.join("objects").join(&h[..2]).join(h)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Remove an object (used by the recovery test to simulate loss).
    pub fn object_path(&self, id: &Id) -> PathBuf {
        self.path(id)
    }
}

impl ObjectStore for FsStore {
    fn put(&self, bytes: &[u8]) -> Result<Id> {
        let id = Id::of(bytes);
        let p = self.path(&id);
        if !p.exists() {
            write_atomic(&p, bytes)?;
        }
        Ok(id)
    }
    fn get(&self, id: &Id) -> Result<Vec<u8>> {
        let bytes = fs::read(self.path(id)).with_context(|| format!("missing object {}", id.short()))?;
        if Id::of(&bytes) != *id {
            bail!("object {} is damaged: contents do not match its name", id.short());
        }
        Ok(bytes)
    }
    fn has(&self, id: &Id) -> bool {
        self.path(id).exists()
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        // E27: flush the bytes to the disk before the rename, so a power cut cannot leave the
        // new name pointing at an empty file.
        use std::io::Write;
        let mut f = fs::File::create(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?;
        f.write_all(bytes).with_context(|| format!("cannot write {}", tmp.display()))?;
        f.sync_all().with_context(|| format!("cannot write {}", tmp.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// Objects in one embedded ACID database file (redb, pure Rust; iroh-blobs already depends on it).
/// Writes are buffered and committed together by `flush`, so one save or one pull is one
/// transaction: measured 30-70x faster than one file per object, and crash-consistent (a crash
/// before `flush` loses only the uncommitted save; refs move only after `flush`).
pub struct RedbStore {
    db: redb::Database,
    pending: Mutex<HashMap<Id, Vec<u8>>>,
}

const OBJECTS: redb::TableDefinition<&[u8; 32], &[u8]> = redb::TableDefinition::new("objects");

impl RedbStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(dir) = path.as_ref().parent() {
            fs::create_dir_all(dir)?;
        }
        let db = redb::Database::create(path.as_ref()).with_context(|| format!("cannot open {}", path.as_ref().display()))?;
        let tx = db.begin_write()?;
        tx.open_table(OBJECTS)?;
        tx.commit()?;
        Ok(Self { db, pending: Mutex::new(HashMap::new()) })
    }

    /// Damage one stored object in place (recovery tests only).
    pub fn damage_for_test(&self, id: &Id) -> Result<()> {
        let mut b = self.get_unverified(id)?;
        let i = b.len() / 2;
        b[i] ^= 0x20;
        let tx = self.db.begin_write()?;
        tx.open_table(OBJECTS)?.insert(&id.0, b.as_slice())?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove(&self, id: &Id) -> Result<()> {
        let tx = self.db.begin_write()?;
        tx.open_table(OBJECTS)?.remove(&id.0)?;
        tx.commit()?;
        Ok(())
    }

    fn get_unverified(&self, id: &Id) -> Result<Vec<u8>> {
        if let Some(b) = self.pending.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
            return Ok(b.clone());
        }
        use redb::ReadableDatabase;
        let tx = self.db.begin_read()?;
        let t = tx.open_table(OBJECTS)?;
        let v = t.get(&id.0)?.with_context(|| format!("missing object {}", id.short()))?;
        Ok(v.value().to_vec())
    }

    /// Every stored id with whether its bytes still match (integrity scan).
    pub fn scan(&self) -> Result<(u64, Vec<Id>)> {
        use redb::{ReadableDatabase, ReadableTable};
        let tx = self.db.begin_read()?;
        let t = tx.open_table(OBJECTS)?;
        let (mut n, mut bad) = (0u64, Vec::new());
        for row in t.iter()? {
            let (k, v) = row?;
            n += 1;
            let id = Id(*k.value());
            if Id::of(v.value()) != id {
                bad.push(id);
            }
        }
        Ok((n, bad))
    }
}

impl ObjectStore for RedbStore {
    fn put(&self, bytes: &[u8]) -> Result<Id> {
        let id = Id::of(bytes);
        let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if !p.contains_key(&id) && !self.has_committed(&id) {
            p.insert(id, bytes.to_vec());
        }
        Ok(id)
    }
    fn get(&self, id: &Id) -> Result<Vec<u8>> {
        let b = self.get_unverified(id)?;
        if Id::of(&b) != *id {
            bail!("object {} is damaged: contents do not match its name", id.short());
        }
        Ok(b)
    }
    fn has(&self, id: &Id) -> bool {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).contains_key(id) || self.has_committed(id)
    }
    fn flush(&self) -> Result<()> {
        let batch: Vec<(Id, Vec<u8>)> = self.pending.lock().unwrap_or_else(|e| e.into_inner()).drain().collect();
        if batch.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut t = tx.open_table(OBJECTS)?;
            for (id, b) in &batch {
                t.insert(&id.0, b.as_slice())?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

impl RedbStore {
    fn has_committed(&self, id: &Id) -> bool {
        use redb::ReadableDatabase;
        (|| -> Result<bool> {
            let tx = self.db.begin_read()?;
            let t = tx.open_table(OBJECTS)?;
            Ok(t.get(&id.0)?.is_some())
        })()
        .unwrap_or(false)
    }
}
