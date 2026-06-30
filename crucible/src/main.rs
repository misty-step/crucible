//! Crucible CLI — evaluate a Cerberus review run against a Daedalus answer key.
//!
//! Two subcommands over the deterministic core:
//!
//! - `crucible adapt <artifact.json> [--json]` projects every Cerberus finding
//!   onto a Daedalus answer-key row and prints the rows. This is an inspection
//!   view of the adapter, faithful to its **total, order-preserving** contract:
//!   every finding yields one row, even an unanchored one. (No `schema_valid`
//!   filtering here — `adapt` shows the raw projection; `grade` is where the
//!   pre-grader's validity filter applies.)
//! - `crucible grade --artifact <a.json> --key <key.json> [--json]` runs the
//!   deterministic pre-grader — drop schema-invalid findings, project, dedup the
//!   key, then [`grade`] — and reports matched / disputed / missed counts plus a
//!   Wilson 95% interval on the match rate `matched / (matched + missed)` (recall
//!   over the key: of the rows the key expected, how many the review found). It
//!   also reports `recoverable_misses` — missed key rows a disputed finding
//!   agrees with on location but not category — so the recall reads as a
//!   category-strict pre-adjudication floor, not a final rate.
//!
//! `--json` emits a stable serde object; the default is a human-readable table.
//! Both subcommands exit `0` on success and non-zero on a load/parse failure.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use crucible_core::{
    dedup, findings_from_artifact, grade, proportion, recoverable_misses, schema_valid,
    to_key_findings, wilson_interval, AnswerKey, GradeResult, KeyFinding,
};
use serde::Serialize;

/// Standard-normal quantile for a two-sided 95% interval.
const Z_95: f64 = 1.96;
/// The confidence level [`Z_95`] corresponds to, surfaced in reports.
const CONFIDENCE: f64 = 0.95;
/// Max width of the rendered description column before truncation.
const DESC_WIDTH: usize = 56;

/// Score a Cerberus review run against a Daedalus answer key.
#[derive(Debug, Parser)]
#[command(name = "crucible", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Project a Cerberus artifact's findings onto Daedalus answer-key rows.
    Adapt {
        /// Path to the Cerberus review artifact JSON.
        #[arg(value_name = "ARTIFACT")]
        artifact: PathBuf,
        /// Emit a stable JSON object instead of a human-readable table.
        #[arg(long)]
        json: bool,
    },
    /// Grade a Cerberus artifact against a Daedalus answer key.
    Grade {
        /// Path to the Cerberus review artifact JSON.
        #[arg(long, value_name = "PATH")]
        artifact: PathBuf,
        /// Path to the Daedalus answer key JSON (`solution/findings.json`).
        #[arg(long, value_name = "PATH")]
        key: PathBuf,
        /// Emit a stable JSON object instead of a human-readable table.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Adapt { artifact, json } => run_adapt(&artifact, json),
        Command::Grade {
            artifact,
            key,
            json,
        } => run_grade(&artifact, &key, json),
    }
}

/// `crucible adapt`: map every finding in the artifact and print the rows.
fn run_adapt(artifact: &Path, json: bool) -> anyhow::Result<()> {
    let findings = findings_from_artifact(artifact)
        .with_context(|| format!("loading artifact {}", artifact.display()))?;
    let rows = to_key_findings(&findings);

    if json {
        let report = AdaptReport {
            artifact: artifact.display().to_string(),
            count: rows.len(),
            findings: &rows,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_adapt_table(artifact, &rows);
    }
    Ok(())
}

/// `crucible grade`: run the deterministic pre-grader and report the result.
fn run_grade(artifact: &Path, key_path: &Path, json: bool) -> anyhow::Result<()> {
    let candidates = candidate_rows(artifact)?;
    let answer_key = AnswerKey::from_path(key_path)
        .with_context(|| format!("loading answer key {}", key_path.display()))?;
    let key_rows = dedup(answer_key.findings);

    let result = grade(&candidates, &key_rows);
    let match_rate = MatchRate::from_grade(&result);
    let recoverable = recoverable_misses(&result);

    if json {
        let report = GradeReport {
            artifact: artifact.display().to_string(),
            key: key_path.display().to_string(),
            matched: result.matched.len(),
            disputed: result.disputed.len(),
            missed: result.missed.len(),
            recoverable_misses: recoverable,
            match_rate,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_grade_summary(artifact, key_path, &result, &match_rate, recoverable);
    }
    Ok(())
}

/// Candidate side of a grade: load findings, drop schema-invalid ones (per the
/// [`grade`] contract — it does no filtering itself), then project the survivors
/// onto answer-key rows.
fn candidate_rows(artifact: &Path) -> anyhow::Result<Vec<KeyFinding>> {
    let findings = findings_from_artifact(artifact)
        .with_context(|| format!("loading artifact {}", artifact.display()))?;
    let valid: Vec<_> = findings.into_iter().filter(schema_valid).collect();
    Ok(to_key_findings(&valid))
}

/// The match-rate point estimate and its Wilson interval, with the raw counts
/// kept so a consumer can tell a true zero rate apart from "no key rows" (`n == 0`).
#[derive(Debug, Serialize)]
struct MatchRate {
    /// Numerator: matched count.
    successes: u64,
    /// Denominator: `matched + missed`.
    n: u64,
    /// Point estimate `successes / n` (`0.0` when `n == 0`).
    point: f64,
    /// Lower Wilson bound.
    lower: f64,
    /// Upper Wilson bound.
    upper: f64,
    /// Standard-normal quantile used for the interval.
    z: f64,
    /// Confidence level `z` corresponds to.
    confidence: f64,
}

impl MatchRate {
    fn from_grade(result: &GradeResult) -> Self {
        let successes = result.matched.len() as u64;
        let n = successes + result.missed.len() as u64;
        let (lower, upper) = wilson_interval(successes, n, Z_95);
        MatchRate {
            successes,
            n,
            point: proportion(successes, n),
            lower,
            upper,
            z: Z_95,
            confidence: CONFIDENCE,
        }
    }
}

/// Stable JSON shape for `adapt --json`.
#[derive(Serialize)]
struct AdaptReport<'a> {
    artifact: String,
    count: usize,
    findings: &'a [KeyFinding],
}

/// Stable JSON shape for `grade --json`.
#[derive(Serialize)]
struct GradeReport {
    artifact: String,
    key: String,
    matched: usize,
    disputed: usize,
    missed: usize,
    /// Missed key rows that share a location with a disputed finding — correct
    /// locations the category-strict matcher could not confirm across the
    /// Cerberus/Daedalus vocabularies, recoverable by a downstream judge. Keeps
    /// the match rate from being read as a final recall.
    recoverable_misses: usize,
    match_rate: MatchRate,
}

/// Render the mapped answer-key rows as an aligned table.
fn print_adapt_table(artifact: &Path, rows: &[KeyFinding]) {
    println!("adapt {}", artifact.display());
    println!("{} mapped finding(s)\n", rows.len());
    if rows.is_empty() {
        println!("(no findings)");
        return;
    }

    let location: Vec<String> = rows.iter().map(location_label).collect();
    let severity: Vec<String> = rows.iter().map(|r| r.severity.clone()).collect();
    let category: Vec<String> = rows.iter().map(|r| r.category.clone()).collect();
    let description: Vec<String> = rows
        .iter()
        .map(|r| first_line_truncated(&r.description, DESC_WIDTH))
        .collect();

    let lw = column_width("LOCATION", &location);
    let sw = column_width("SEVERITY", &severity);
    let cw = column_width("CATEGORY", &category);

    println!(
        "{:<lw$}  {:<sw$}  {:<cw$}  DESCRIPTION",
        "LOCATION", "SEVERITY", "CATEGORY"
    );
    for i in 0..rows.len() {
        println!(
            "{:<lw$}  {:<sw$}  {:<cw$}  {}",
            location[i], severity[i], category[i], description[i]
        );
    }
}

/// Render the grade partition and the match-rate interval.
fn print_grade_summary(
    artifact: &Path,
    key: &Path,
    result: &GradeResult,
    rate: &MatchRate,
    recoverable: usize,
) {
    println!("crucible grade");
    println!("  artifact   {}", artifact.display());
    println!("  key        {}\n", key.display());
    println!("  matched    {}", result.matched.len());
    println!("  disputed   {}", result.disputed.len());
    println!("  missed     {}\n", result.missed.len());

    if rate.n == 0 {
        println!("  match rate  n/a — no key rows to match");
    } else {
        println!(
            "  match rate  {:.1}%   {:.0}% CI [{:.1}%, {:.1}%]   (Wilson, matched/(matched+missed) = {}/{})",
            rate.point * 100.0,
            rate.confidence * 100.0,
            rate.lower * 100.0,
            rate.upper * 100.0,
            rate.successes,
            rate.n,
        );
    }

    if recoverable > 0 {
        println!(
            "\n  note  {recoverable} missed key row(s) share a location with a disputed finding (category vocabulary mismatch); this recall is a category-strict pre-adjudication floor, not a final rate"
        );
    }
}

/// `file:line`, or a clear sentinel for the adapter's unanchored row.
fn location_label(row: &KeyFinding) -> String {
    if row.file.is_empty() {
        "(unanchored)".to_string()
    } else {
        format!("{}:{}", row.file, row.line)
    }
}

/// Widest of the header and every cell, for left-aligned columns.
fn column_width(header: &str, cells: &[String]) -> usize {
    cells
        .iter()
        .map(String::len)
        .chain(std::iter::once(header.len()))
        .max()
        .unwrap_or(0)
}

/// First line of `s`, trimmed, truncated to `max` chars with an ellipsis.
fn first_line_truncated(s: &str, max: usize) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    if first.chars().count() <= max {
        return first.to_string();
    }
    let take = max.saturating_sub(1).max(1);
    let head: String = first.chars().take(take).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crucible_core::Match;

    fn kf(file: &str, line: u32) -> KeyFinding {
        KeyFinding {
            file: file.to_string(),
            line,
            category: "security".to_string(),
            severity: "blocking".to_string(),
            description: "d".to_string(),
        }
    }

    #[test]
    fn match_rate_from_empty_grade_is_na_shaped() {
        // The n == 0 case (empty key) the CLI renders as "n/a": no key rows, so
        // the point estimate is a documented 0.0 the caller distinguishes via n.
        let result = GradeResult {
            matched: Vec::new(),
            disputed: Vec::new(),
            missed: Vec::new(),
        };
        let rate = MatchRate::from_grade(&result);
        assert_eq!(rate.n, 0);
        assert_eq!(rate.successes, 0);
        assert_eq!(rate.point, 0.0);
        assert_eq!(rate.lower, 0.0);
        assert_eq!(rate.upper, 0.0);
    }

    #[test]
    fn match_rate_point_is_matched_over_matched_plus_missed() {
        // 1 matched + 1 missed -> recall 0.5 over n = 2; disputed does not enter
        // the denominator.
        let result = GradeResult {
            matched: vec![Match {
                candidate: kf("a.rs", 10),
                key: kf("a.rs", 10),
            }],
            disputed: vec![kf("z.rs", 99)],
            missed: vec![kf("b.rs", 20)],
        };
        let rate = MatchRate::from_grade(&result);
        assert_eq!(rate.successes, 1);
        assert_eq!(rate.n, 2);
        assert!((rate.point - 0.5).abs() < 1e-9);
        assert!(rate.lower < rate.point && rate.point < rate.upper);
    }

    #[test]
    fn first_line_truncated_keeps_short_first_line() {
        assert_eq!(first_line_truncated("headline\n\nbody", 56), "headline");
    }

    #[test]
    fn first_line_truncated_adds_ellipsis_when_too_long() {
        let out = first_line_truncated("abcdefghij", 5);
        assert!(out.ends_with('…'), "truncated output ends with an ellipsis");
        assert_eq!(out.chars().count(), 5, "respects the max width");
    }

    #[test]
    fn location_label_marks_the_unanchored_sentinel() {
        assert_eq!(location_label(&kf("src/x.rs", 7)), "src/x.rs:7");
        assert_eq!(location_label(&kf("", 0)), "(unanchored)");
    }
}
