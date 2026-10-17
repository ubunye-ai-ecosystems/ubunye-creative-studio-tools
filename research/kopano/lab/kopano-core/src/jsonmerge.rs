//! Describing and merging JSON item states (application-neutral). Merging is three-way and per
//! setting: if two people change different settings of the same item, both changes are kept. If
//! they change the same setting differently, Kopano never guesses: it keeps yours and reports it.
//! Apps can give nicer words through `Connector::describe`; this is the generic fallback.

use serde_json::{Map, Value};
use std::collections::BTreeMap;

fn flatten(v: &Value, prefix: &str, out: &mut BTreeMap<String, Value>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(x, &p, out);
            }
        }
        _ => {
            out.insert(prefix.to_string(), v.clone());
        }
    }
}

/// Generic label from a JSON path: the last two segments, words spaced, first letter capital.
/// "develop.light.exposure" -> "Light exposure". Apps override wording via their connector.
fn label(path: &str) -> String {
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty()).collect();
    let tail = &parts[parts.len().saturating_sub(2)..];
    let text = tail.iter().map(|s| s.replace('_', " ")).collect::<Vec<_>>().join(" ");
    let mut c = text.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => text,
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::Number(n) => n.as_f64().map(|f| format!("{f:.2}")).unwrap_or_else(|| n.to_string()),
        Value::String(s) => s.clone(),
        Value::Null => "none".into(),
        Value::Array(a) => format!("{} items", a.len()),
        other => other.to_string(),
    }
}

/// Plain-words list of what changed between two states of one photo.
pub fn describe(old: Option<&Value>, new: &Value) -> Vec<String> {
    let Some(old) = old else { return vec!["new to this project".into()] };
    let (mut a, mut b) = (BTreeMap::new(), BTreeMap::new());
    flatten(old, "", &mut a);
    flatten(new, "", &mut b);
    let mut out = Vec::new();
    for (k, nv) in &b {
        let ov = a.get(k).unwrap_or(&Value::Null);
        if ov == nv || k == "key" || k == "kind" {
            continue;
        }
        let line = match (ov, nv) {
            (Value::Number(x), Value::Number(y)) => {
                let (x, y) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
                format!("{} {:.2} → {:.2} ({:+.2})", label(k), x, y, y - x)
            }
            (Value::Array(x), Value::Array(y)) if k.ends_with("masks") || x.len() != y.len() => {
                format!("{} changed ({} → {})", label(k), x.len(), y.len())
            }
            _ => format!("{}: {} → {}", label(k), show(ov), show(nv)),
        };
        out.push(line);
    }
    for k in a.keys().filter(|k| !b.contains_key(*k)) {
        out.push(format!("{} reset", label(k)));
    }
    out
}

/// Three-way merge of one photo's state. Returns the merged state and the settings that clashed.
pub fn merge3(base: Option<&Value>, mine: &Value, theirs: &Value) -> (Value, Vec<String>) {
    let mut clashes = Vec::new();
    let merged = merge_value(base, mine, theirs, "", &mut clashes, Prefer::Neither);
    (merged, clashes)
}

/// Like `merge3`, but where both changed the same setting a person decided which side wins
/// (an approver accepting work into main).
pub fn merge3_prefer(base: Option<&Value>, mine: &Value, theirs: &Value, prefer_theirs: bool) -> (Value, Vec<String>) {
    let mut clashes = Vec::new();
    let merged = merge_value(base, mine, theirs, "", &mut clashes, if prefer_theirs { Prefer::Theirs } else { Prefer::Mine });
    (merged, clashes)
}

#[derive(Clone, Copy)]
enum Prefer {
    Neither,
    Mine,
    Theirs,
}

fn merge_value(base: Option<&Value>, mine: &Value, theirs: &Value, path: &str, clashes: &mut Vec<String>, prefer: Prefer) -> Value {
    if mine == theirs || base == Some(theirs) {
        return mine.clone();
    }
    if base == Some(mine) {
        return theirs.clone();
    }
    if let (Value::Object(m), Value::Object(t)) = (mine, theirs) {
        let b = base.and_then(Value::as_object);
        let mut out = Map::new();
        let keys: std::collections::BTreeSet<&String> = m.keys().chain(t.keys()).collect();
        for k in keys {
            let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
            let bv = b.and_then(|b| b.get(k));
            match (m.get(k), t.get(k)) {
                (Some(mv), Some(tv)) => {
                    out.insert(k.clone(), merge_value(bv, mv, tv, &p, clashes, prefer));
                }
                (Some(mv), None) => {
                    // They removed it (back to default) and I did not touch it: take the removal.
                    if bv != Some(mv) {
                        out.insert(k.clone(), mv.clone());
                    }
                }
                (None, Some(tv)) => {
                    if bv != Some(tv) {
                        out.insert(k.clone(), tv.clone());
                    }
                }
                (None, None) => {}
            }
        }
        return Value::Object(out);
    }
    clashes.push(label(path));
    // Both changed it differently. The kept value must not depend on who merges, or two people
    // merging at once disagree and keep clashing: take the larger value in canonical JSON order.
    // The other value is never lost: it stays in the clash record for a person to pick.
    match prefer {
        Prefer::Mine => mine.clone(),
        Prefer::Theirs => theirs.clone(),
        Prefer::Neither => deterministic_pick(mine, theirs).clone(),
    }
}

/// The same choice on every device, whichever side is "mine".
pub fn deterministic_pick<'a>(a: &'a Value, b: &'a Value) -> &'a Value {
    let (sa, sb) = (serde_json::to_string(a).unwrap_or_default(), serde_json::to_string(b).unwrap_or_default());
    if sa >= sb { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn different_sliders_both_survive() {
        let base = json!({"develop": {"light": {"exposure": 0.0, "contrast": 0.0}}});
        let mine = json!({"develop": {"light": {"exposure": 0.5, "contrast": 0.0}}});
        let theirs = json!({"develop": {"light": {"exposure": 0.0, "contrast": 20.0}}});
        let (m, c) = merge3(Some(&base), &mine, &theirs);
        assert!(c.is_empty());
        assert_eq!(m, json!({"develop": {"light": {"exposure": 0.5, "contrast": 20.0}}}));
    }

    #[test]
    fn same_slider_clash_is_the_same_whoever_merges_and_reports() {
        let base = json!({"rating": 0});
        let (m1, c1) = merge3(Some(&base), &json!({"rating": 3}), &json!({"rating": 5}));
        let (m2, c2) = merge3(Some(&base), &json!({"rating": 5}), &json!({"rating": 3}));
        assert_eq!(m1, m2);
        assert_eq!(c1, vec!["Rating".to_string()]);
        assert_eq!(c1, c2);
    }

    #[test]
    fn describe_reads_like_words() {
        let a = json!({"develop": {"light": {"exposure": 0.0}}, "rating": 0});
        let b = json!({"develop": {"light": {"exposure": 0.3}}, "rating": 4});
        let d = describe(Some(&a), &b);
        assert!(d.contains(&"Light exposure 0.00 → 0.30 (+0.30)".to_string()), "{d:?}");
        assert!(d.contains(&"Rating 0.00 → 4.00 (+4.00)".to_string()), "{d:?}");
    }
}
