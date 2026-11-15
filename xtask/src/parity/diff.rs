//! Structured diffs between observation trees.

use serde::Serialize;
use serde_json::Value;

/// One difference at a JSON path.
#[derive(Debug, Clone, Serialize)]
pub struct Difference {
    /// Dotted path, e.g. `status_47.parsed.players.max` or `log[3]`.
    pub path: String,
    /// The vanilla value (`null` if missing).
    pub expected: Value,
    /// The krol value (`null` if missing).
    pub actual: Value,
}

/// Absolute numeric tolerances per observation path (each also covers everything
/// below it).
pub type Tolerances = std::collections::BTreeMap<String, f64>;

/// Puts every run of directly adjacent packets whose types are all in `group` into a
/// canonical order, in each bot stream of `observations` (objects with parallel
/// `sequence` and `packets` arrays).
pub fn canonicalize_unordered(observations: &mut Value, group: &[String]) {
    let Some(streams) = observations.as_object_mut() else {
        return;
    };
    for stream in streams.values_mut() {
        let Some(obj) = stream.as_object_mut() else {
            continue;
        };
        let (Some(Value::Array(sequence)), Some(Value::Array(packets))) =
            (obj.get("sequence").cloned(), obj.get("packets").cloned())
        else {
            continue;
        };
        if sequence.len() != packets.len() {
            continue;
        }
        let in_group = |v: &Value| v.as_str().is_some_and(|t| group.iter().any(|g| g == t));
        let mut pairs: Vec<(Value, Value)> = sequence.into_iter().zip(packets).collect();
        let mut start = 0;
        while start < pairs.len() {
            let mut end = start;
            while end < pairs.len() && in_group(&pairs[end].0) {
                end += 1;
            }
            if end > start + 1 {
                pairs[start..end].sort_by_key(|(t, p)| format!("{t}{p}"));
            }
            start = end.max(start + 1);
        }
        let (sequence, packets): (Vec<Value>, Vec<Value>) = pairs.into_iter().unzip();
        obj.insert("sequence".into(), Value::Array(sequence));
        obj.insert("packets".into(), Value::Array(packets));
    }
}

/// Drops every packet of a type in `types` that equals the packet right before it, in
/// each bot stream of `observations`.
pub fn collapse_repeats(observations: &mut Value, types: &[String]) {
    let Some(streams) = observations.as_object_mut() else {
        return;
    };
    for stream in streams.values_mut() {
        let Some(obj) = stream.as_object_mut() else {
            continue;
        };
        let (Some(Value::Array(sequence)), Some(Value::Array(packets))) =
            (obj.get("sequence").cloned(), obj.get("packets").cloned())
        else {
            continue;
        };
        if sequence.len() != packets.len() {
            continue;
        }
        let mut kept: Vec<(Value, Value)> = Vec::new();
        for (t, p) in sequence.into_iter().zip(packets) {
            let listed = t.as_str().is_some_and(|t| types.iter().any(|x| x == t));
            if listed && kept.last().is_some_and(|(lt, lp)| *lt == t && *lp == p) {
                continue;
            }
            kept.push((t, p));
        }
        let (sequence, packets): (Vec<Value>, Vec<Value>) = kept.into_iter().unzip();
        obj.insert("sequence".into(), Value::Array(sequence));
        obj.insert("packets".into(), Value::Array(packets));
    }
}

/// Computes the differences between `expected` and `actual`, skipping ignored paths
/// (an ignored path also hides everything below it) and accepting numbers within the
/// tolerance of their path.
pub fn diff(
    expected: &Value,
    actual: &Value,
    ignore: &[String],
    tolerances: &Tolerances,
) -> Vec<Difference> {
    let mut out = Vec::new();
    let ctx = Ctx { ignore, tolerances };
    walk(&ctx, "", expected, actual, &mut out);
    out
}

struct Ctx<'a> {
    ignore: &'a [String],
    tolerances: &'a Tolerances,
}

/// Whether `path` is `base` or lies below it.
fn under(path: &str, base: &str) -> bool {
    path == base
        || path
            .strip_prefix(base)
            .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
}

fn is_ignored(path: &str, ignore: &[String]) -> bool {
    ignore.iter().any(|i| under(path, i))
}

/// The tolerance of the most specific entry covering `path`.
fn tolerance(path: &str, tolerances: &Tolerances) -> Option<f64> {
    tolerances
        .iter()
        .filter(|(base, _)| under(path, base))
        .max_by_key(|(base, _)| base.len())
        .map(|(_, t)| *t)
}

/// A JSON number, or an SNBT-style number string such as `0.5f` or `12L`.
fn numeric(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t = s.trim_end_matches(['b', 's', 'L', 'f', 'd']);
            t.parse().ok()
        }
        _ => None,
    }
}

fn join(base: &str, key: &str) -> String {
    if base.is_empty() {
        key.to_owned()
    } else {
        format!("{base}.{key}")
    }
}

fn walk(ctx: &Ctx, path: &str, expected: &Value, actual: &Value, out: &mut Vec<Difference>) {
    if is_ignored(path, ctx.ignore) {
        return;
    }
    let tol = tolerance(path, ctx.tolerances);
    if let Some(t) = tol
        && let (Some(e), Some(a)) = (numeric(expected), numeric(actual))
        && (e - a).abs() <= t
    {
        return;
    }
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                walk(
                    ctx,
                    &join(path, k),
                    ev,
                    a.get(k).unwrap_or(&Value::Null),
                    out,
                );
            }
            for (k, av) in a {
                if !e.contains_key(k) {
                    walk(ctx, &join(path, k), &Value::Null, av, out);
                }
            }
        }
        (Value::Array(e), Value::Array(a))
            if tol.is_none() && is_line_list(e) && is_line_list(a) =>
        {
            line_diff(path, e, a, out);
        }
        (Value::Array(e), Value::Array(a)) => {
            for i in 0..e.len().max(a.len()) {
                walk(
                    ctx,
                    &format!("{path}[{i}]"),
                    e.get(i).unwrap_or(&Value::Null),
                    a.get(i).unwrap_or(&Value::Null),
                    out,
                );
            }
        }
        _ if expected == actual => {}
        _ => out.push(Difference {
            path: path.to_owned(),
            expected: expected.clone(),
            actual: actual.clone(),
        }),
    }
}

fn is_line_list(v: &[Value]) -> bool {
    v.iter().all(Value::is_string)
}

/// Diffs two lists of lines with an LCS alignment so one inserted line does not make
/// every following line differ. Reports runs of missing/extra lines.
fn line_diff(path: &str, e: &[Value], a: &[Value], out: &mut Vec<Difference>) {
    let n = e.len();
    let m = a.len();
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if e[i] == a[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && e[i] == a[j] {
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            out.push(Difference {
                path: format!("{path}[+{j}]"),
                expected: Value::Null,
                actual: a[j].clone(),
            });
            j += 1;
        } else {
            out.push(Difference {
                path: format!("{path}[{i}]"),
                expected: e[i].clone(),
                actual: Value::Null,
            });
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repeats_collapse() {
        let types = vec!["a".to_owned()];
        let stream = |seq: &[(&str, i32)]| {
            json!({ "bot": {
                "sequence": seq.iter().map(|(t, _)| t).collect::<Vec<_>>(),
                "packets": seq.iter().map(|(t, v)| json!({ "packet": t, "v": v })).collect::<Vec<_>>(),
            }})
        };
        let mut x = stream(&[
            ("a", 1),
            ("a", 1),
            ("a", 1),
            ("a", 2),
            ("b", 0),
            ("b", 0),
            ("a", 2),
        ]);
        collapse_repeats(&mut x, &types);
        assert_eq!(
            x,
            stream(&[("a", 1), ("a", 2), ("b", 0), ("b", 0), ("a", 2)])
        );
    }

    #[test]
    fn unordered_runs_are_canonical() {
        let group = vec!["a".to_owned(), "b".to_owned()];
        let stream = |seq: &[&str]| {
            json!({ "bot": {
                "sequence": seq,
                "packets": seq.iter().map(|t| json!({ "packet": t })).collect::<Vec<_>>(),
            }})
        };
        let mut x = stream(&["c", "b", "a", "c", "b"]);
        let mut y = stream(&["c", "a", "b", "c", "b"]);
        canonicalize_unordered(&mut x, &group);
        canonicalize_unordered(&mut y, &group);
        assert_eq!(x, y);
        // Packets outside the group still keep their place.
        let mut z = stream(&["b", "c", "a"]);
        canonicalize_unordered(&mut z, &group);
        assert_eq!(z, stream(&["b", "c", "a"]));
    }

    #[test]
    fn structured_paths_and_ignores() {
        let e = json!({"a": {"b": 1, "c": 2}, "x": [1, 2]});
        let a = json!({"a": {"b": 1, "c": 3}, "x": [1], "y": true});
        let d = diff(&e, &a, &["y".to_owned()], &Tolerances::new());
        let paths: Vec<_> = d.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, ["a.c", "x[1]"]);
    }

    #[test]
    fn line_lists_align() {
        let e = json!(["a", "b", "c"]);
        let a = json!(["a", "x", "b", "c"]);
        let d = diff(&e, &a, &[], &Tolerances::new());
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path, "[+1]");
    }

    #[test]
    fn tolerances_cover_numbers_below_a_path() {
        let e = json!({"f": {"x": "0.5f", "m": ["1.0d", "2.0d"]}, "n": 10});
        let a = json!({"f": {"x": "0.52f", "m": ["1.01d", "2.0d"]}, "n": 13});
        let mut t = Tolerances::new();
        t.insert("f".to_owned(), 0.05);
        t.insert("n".to_owned(), 2.0);
        let d = diff(&e, &a, &[], &t);
        let paths: Vec<_> = d.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, ["n"]);
    }
}
