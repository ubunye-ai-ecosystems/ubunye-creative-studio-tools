//! Merging two lines of work. Item by item, using the manifest diff against the common ancestor:
//!
//! | base → mine | base → theirs | result |
//! |---|---|---|
//! | unchanged | changed | take theirs |
//! | changed | unchanged | keep mine |
//! | changed | changed the same way | keep (identical) |
//! | changed | changed differently, kind is mergeable JSON | merge setting by setting; a setting both changed differently is a clash |
//! | changed | changed differently, not mergeable (binary, unknown kind, delete vs edit) | clash |
//!
//! A clash never loses data: mine stays in place and a conflict record holding both sides is
//! added to the project (lane `kopano.conflicts`), so it syncs to everyone until a person picks.

use crate::jsonmerge;
use crate::project::Item;
use crate::store::{Id, ObjectStore};
use crate::tree;
use anyhow::Result;
use serde_json::json;
use std::collections::BTreeMap;

pub const CONFLICT_LANE: &str = "kopano.conflicts";
pub const CONFLICT_PREFIX: &str = "kopano/conflict/";

#[derive(Debug, Clone)]
pub struct Conflict {
    pub key: String,
    pub fields: Vec<String>,
    pub mine: Option<Id>,
    pub theirs: Option<Id>,
}

pub struct MergeOutcome {
    pub root: Option<Id>,
    pub conflicts: Vec<Conflict>,
    /// Keys whose item differs from mine after the merge (what the app must apply).
    pub changed: Vec<String>,
}

/// Which item kinds may be merged setting by setting. Supplied by the connector, never guessed.
pub trait MergePolicy {
    fn mergeable(&self, kind: &str) -> bool;
}

impl<F: Fn(&str) -> bool> MergePolicy for F {
    fn mergeable(&self, kind: &str) -> bool {
        self(kind)
    }
}

pub fn merge_trees(store: &dyn ObjectStore, base: Option<Id>, mine: Option<Id>, theirs: Option<Id>, policy: &dyn MergePolicy) -> Result<MergeOutcome> {
    let mine_changes: BTreeMap<String, Option<Id>> = tree::diff(store, base, mine)?.into_iter().map(|(k, _, m)| (k, m)).collect();
    let mut changes: Vec<tree::Change> = Vec::new();
    let mut conflicts = Vec::new();
    let mut changed = Vec::new();
    for (key, b, t) in tree::diff(store, base, theirs)? {
        let Some(m) = mine_changes.get(&key).copied() else {
            changes.push((key.clone(), t));
            changed.push(key);
            continue;
        };
        if m == t {
            continue;
        }
        // Both sides changed this item differently.
        let mut clash_fields = vec!["whole item".to_string()];
        if let (Some(mi), Some(ti)) = (m, t) {
            let (mi_item, ti_item) = (Item::get(store, &mi)?, Item::get(store, &ti)?);
            let base_item = b.map(|id| Item::get(store, &id)).transpose()?;
            let same_shape = mi_item.kind == ti_item.kind && mi_item.lane == ti_item.lane && mi_item.schema == ti_item.schema;
            if same_shape && policy.mergeable(&mi_item.kind) {
                let (state, mut fields) = jsonmerge::merge3(base_item.as_ref().map(|i| &i.state), &mi_item.state, &ti_item.state);
                let assets = if mi_item.assets == ti_item.assets || base_item.as_ref().map(|i| &i.assets) == Some(&ti_item.assets) {
                    mi_item.assets.clone()
                } else if base_item.as_ref().map(|i| &i.assets) == Some(&mi_item.assets) {
                    ti_item.assets.clone()
                } else {
                    fields.push("Media".into());
                    let (ma, ta) = (json!(mi_item.assets), json!(ti_item.assets));
                    if jsonmerge::deterministic_pick(&ma, &ta) == &ma { mi_item.assets.clone() } else { ti_item.assets.clone() }
                };
                let merged = Item { state, assets, ..mi_item.clone() };
                let merged_id = merged.put(store)?;
                if merged_id != mi {
                    changes.push((key.clone(), Some(merged_id)));
                    changed.push(key.clone());
                }
                if fields.is_empty() {
                    continue;
                }
                clash_fields = fields;
            }
        }
        // Whole-item clash (cannot merge setting by setting, or deleted on one side and edited on
        // the other): the same outcome on every device. Keeping data beats deleting it; otherwise
        // the larger id wins. Mergeable clashes were already resolved deterministically above.
        if clash_fields == ["whole item".to_string()] {
            let keep = match (m, t) {
                (None, t) => t,
                (m, None) => m,
                (Some(a), Some(b)) => Some(if a.hex() >= b.hex() { a } else { b }),
            };
            if keep != m {
                changes.push((key.clone(), keep));
                changed.push(key.clone());
            }
        }
        // The record names both sides in a fixed order, so every device writes the same record.
        let (a, b) = if m.map(|i| i.hex()) <= t.map(|i| i.hex()) { (m, t) } else { (t, m) };
        let record = Item {
            kind: "kopano.conflict".into(),
            lane: CONFLICT_LANE.into(),
            schema: 1,
            state: json!({
                "key": key,
                "fields": clash_fields,
                "mine": a.map(|i| i.hex()),
                "theirs": b.map(|i| i.hex()),
            }),
            assets: vec![],
        };
        let short = |x: Option<Id>| x.map(|i| i.short()).unwrap_or_else(|| "removed".into());
        changes.push((format!("{CONFLICT_PREFIX}{key}/{}-{}", short(a), short(b)), Some(record.put(store)?)));
        conflicts.push(Conflict { key, fields: clash_fields, mine: m, theirs: t });
    }
    let root = tree::update(store, mine, changes)?;
    Ok(MergeOutcome { root, conflicts, changed })
}
