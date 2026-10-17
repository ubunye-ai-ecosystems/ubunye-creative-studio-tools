//! The application connector contract. This is the whole surface an app must implement.
//!
//! In-process it is this trait. Across processes (an app in another language, or a running app
//! that Kopano must not link into) the same five calls travel as JSON-RPC messages; the lab's
//! Light experiment does exactly that over Light's existing control port.
//!
//! Rules every connector must follow (checked by `conformance`):
//! 1. `snapshot` returns only this app's items, each with a stable key that is the same on
//!    every computer (never a local database row id).
//! 2. `apply(key, snapshot()[key])` is a no-op, and `apply` then `snapshot` returns exactly the
//!    item applied (no lossy conversion).
//! 3. Media is referenced by BLAKE3 hash, never embedded in the state.
//! 4. `info().kinds` declares which kinds may be merged setting by setting. Undeclared kinds are
//!    never merged; a concurrent change to them becomes a conflict for a person to resolve.
//! 5. A connector never writes items of a kind it does not declare, so it cannot corrupt another
//!    app's data in a shared project.

use crate::project::Item;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version of this contract. A connector built for a newer major version is refused.
pub const CONTRACT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct KindInfo {
    pub kind: String,
    pub lane: String,
    pub schema: u32,
    pub mergeable: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConnectorInfo {
    pub app: String,
    pub contract: u32,
    pub kinds: Vec<KindInfo>,
}

impl ConnectorInfo {
    pub fn owns(&self, kind: &str) -> bool {
        self.kinds.iter().any(|k| k.kind == kind)
    }
    pub fn mergeable(&self, kind: &str) -> bool {
        self.kinds.iter().any(|k| k.kind == kind && k.mergeable)
    }
}

pub trait Connector {
    fn info(&self) -> ConnectorInfo;
    /// Every item the app currently has, by stable key.
    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>>;
    /// Make the app's item match `item` (or remove it with `None`).
    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()>;
    /// Plain words for a change, for history and review screens.
    fn describe(&self, before: Option<&Item>, after: &Item) -> Vec<String> {
        crate::jsonmerge::describe(before.map(|b| &b.state), &after.state)
    }
}
