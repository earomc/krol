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

/// Computes the differences between `expected` and `actual`, skipping ignored paths
/// (an ignored path also hides everything below it).
pub fn diff(expected: &Value, actual: &Value, ignore: &[String]) -> Vec<Difference> {
    let mut out = Vec::new();
    walk("", expected, actual, ignore, &mut out);
    out
}

fn is_ignored(path: &str, ignore: &[String]) -> bool {
    ignore.iter().any(|i| {
        path == i
            || path
                .strip_prefix(i.as_str())
                .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
    })
}

fn join(base: &str, key: &str) -> String {
    if base.is_empty() {
        key.to_owned()
    } else {
        format!("{base}.{key}")
    }
}

fn walk(
    path: &str,
    expected: &Value,
    actual: &Value,
    ignore: &[String],
    out: &mut Vec<Difference>,
) {
    if is_ignored(path, ignore) {
        return;
    }
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                walk(
                    &join(path, k),
                    ev,
                    a.get(k).unwrap_or(&Value::Null),
                    ignore,
                    out,
                );
            }
            for (k, av) in a {
                if !e.contains_key(k) {
                    walk(&join(path, k), &Value::Null, av, ignore, out);
                }
            }
        }
        (Value::Array(e), Value::Array(a)) if is_line_list(e) && is_line_list(a) => {
            line_diff(path, e, a, out);
        }
        (Value::Array(e), Value::Array(a)) => {
            for i in 0..e.len().max(a.len()) {
                walk(
                    &format!("{path}[{i}]"),
                    e.get(i).unwrap_or(&Value::Null),
                    a.get(i).unwrap_or(&Value::Null),
                    ignore,
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
    fn structured_paths_and_ignores() {
        let e = json!({"a": {"b": 1, "c": 2}, "x": [1, 2]});
        let a = json!({"a": {"b": 1, "c": 3}, "x": [1], "y": true});
        let d = diff(&e, &a, &["y".to_owned()]);
        let paths: Vec<_> = d.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, ["a.c", "x[1]"]);
    }

    #[test]
    fn line_lists_align() {
        let e = json!(["a", "b", "c"]);
        let a = json!(["a", "x", "b", "c"]);
        let d = diff(&e, &a, &[]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path, "[+1]");
    }
}
