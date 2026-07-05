//! Shared save gate for anything that assembles an [`EvalSpec`] and wants to
//! write it to disk: `crucible author` (crucible-942) and `crucible import`
//! (crucible-026). Both converge on the same rule — write the assembled spec
//! to a scratch file, run it through the exact [`crate::validate::validate`]
//! function `crucible validate` calls (never forked), and only rename it
//! into place when the report says `valid`. An invalid assembly is refused
//! with the same `{valid, runnable, errors, warnings}` shape `crucible
//! validate` prints, and leaves no file behind at the real output path.

use std::path::{Path, PathBuf};

use anyhow::Context;
use crucible_core::EvalSpec;

use crate::validate::{self, ValidationReport};

/// Resolve the default output path for an assembled spec when `--out` was
/// not given: `evals/<id-or-task-slug>.json`.
pub(crate) fn resolve_out_path(out: Option<&Path>, spec: &EvalSpec) -> PathBuf {
    if let Some(out) = out {
        return out.to_path_buf();
    }
    let slug = if !spec.id.trim().is_empty() {
        slugify(&spec.id)
    } else {
        slugify(&spec.task)
    };
    Path::new("evals").join(format!("{slug}.json"))
}

/// Lowercase-alnum-and-dash slug, used only for a friendly default `--out`
/// filename when one isn't given — never for anything Crucible reads back
/// structurally.
pub(crate) fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "eval".to_string()
    } else {
        out
    }
}

/// Write the assembled spec to a scratch file beside `out_path`, validate it
/// through the exact `crucible validate` path, and rename it into place iff
/// valid. Returns the report (with `spec` rewritten to `out_path` so the
/// printed report never leaks the scratch filename) and whether the file was
/// actually written.
pub(crate) fn validate_and_maybe_write(
    spec: &EvalSpec,
    out_path: &Path,
) -> anyhow::Result<(ValidationReport, bool)> {
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
    }
    let tmp_path = temp_sibling_path(out_path)?;
    let json = serde_json::to_string_pretty(spec).context("serializing assembled spec")?;
    std::fs::write(&tmp_path, format!("{json}\n"))
        .with_context(|| format!("writing scratch spec {}", tmp_path.display()))?;

    let mut report = match validate::validate(&tmp_path) {
        Ok(report) => report,
        Err(err) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(err);
        }
    };
    // The scratch path only exists to let `validate::validate` run unforked;
    // the report the operator/agent sees should name the real destination.
    report.spec = out_path.display().to_string();

    if report.valid {
        if std::fs::rename(&tmp_path, out_path).is_err() {
            // Cross-device out paths (rare, e.g. --out on a different mount
            // than evals/) can't rename; fall back to copy + remove.
            std::fs::copy(&tmp_path, out_path)
                .with_context(|| format!("writing assembled spec to {}", out_path.display()))?;
            let _ = std::fs::remove_file(&tmp_path);
        }
        Ok((report, true))
    } else {
        let _ = std::fs::remove_file(&tmp_path);
        Ok((report, false))
    }
}

fn temp_sibling_path(out_path: &Path) -> anyhow::Result<PathBuf> {
    let parent = out_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = out_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("spec.json");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    Ok(parent.join(format!(
        ".crucible-author-{}-{nonce}-{name}",
        std::process::id()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_dashes() {
        assert_eq!(slugify("My New Eval v0!"), "my-new-eval-v0");
        assert_eq!(slugify("---"), "eval");
        assert_eq!(slugify(""), "eval");
    }

    #[test]
    fn resolve_out_path_defaults_to_evals_dir_with_id_slug() {
        let mut spec_json = serde_json::json!({"task": "code-review"});
        spec_json["id"] = serde_json::Value::String("My Eval V0".to_string());
        let spec: EvalSpec = serde_json::from_value(spec_json).unwrap();
        let path = resolve_out_path(None, &spec);
        assert_eq!(path, Path::new("evals/my-eval-v0.json"));
    }
}
