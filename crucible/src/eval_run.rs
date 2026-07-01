//! Built-in runnable evals for the factory lane.
//!
//! These are not a general eval registry. They are three concrete, committed
//! fixtures that exercise the code-review wedge end to end and emit the same
//! statistical shape the rest of Crucible promises: every binary score carries a
//! Wilson interval, and every artifact is written to an inspectable output dir.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::ValueEnum;
use crucible_core::{
    adjudications_from_queue, build_queue, extended_expected_key, grade, recoverable_misses,
    render_adjudications_md, AnswerKey, ArenaVersion, ExpectedKey, ExportContext,
};
use serde::Serialize;

use crate::adjudication_panel;
use crate::{candidate_rows, load_key_rows, load_queue, wilson_score, MatchRate};

/// Stable schema id for `crucible run --json` and `<out>/run-report.json`.
pub const RUN_REPORT_SCHEMA: &str = "crucible.run_report.v1";

/// Built-in eval selector for `crucible run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunEval {
    /// Run every built-in eval.
    All,
    /// Grade a real Cerberus artifact against a scorer key.
    CodeReviewDeterministicFloor,
    /// Build the human queue for a recoverable category-vocabulary miss.
    RecoverableAdjudicationQueue,
    /// Apply labels and export an accepted finding into Harbor scorer artifacts.
    HarborExportAcceptance,
}

impl RunEval {
    fn selected(self) -> Vec<Self> {
        match self {
            RunEval::All => vec![
                RunEval::CodeReviewDeterministicFloor,
                RunEval::RecoverableAdjudicationQueue,
                RunEval::HarborExportAcceptance,
            ],
            other => vec![other],
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            RunEval::All => "all",
            RunEval::CodeReviewDeterministicFloor => "code-review-deterministic-floor",
            RunEval::RecoverableAdjudicationQueue => "recoverable-adjudication-queue",
            RunEval::HarborExportAcceptance => "harbor-export-acceptance",
        }
    }
}

impl fmt::Display for RunEval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// Run one or all built-in evals, write their evidence artifacts, and return the
/// summary report.
pub fn run(eval: RunEval, out: &Path) -> anyhow::Result<RunReport> {
    std::fs::create_dir_all(out)
        .with_context(|| format!("creating run output directory {}", out.display()))?;

    let mut evals = Vec::new();
    for selected in eval.selected() {
        let eval_out = out.join(selected.id());
        std::fs::create_dir_all(&eval_out)
            .with_context(|| format!("creating eval output directory {}", eval_out.display()))?;
        evals.push(match selected {
            RunEval::All => unreachable!("RunEval::All expands before dispatch"),
            RunEval::CodeReviewDeterministicFloor => run_code_review_floor(&eval_out)?,
            RunEval::RecoverableAdjudicationQueue => run_recoverable_queue(&eval_out)?,
            RunEval::HarborExportAcceptance => run_harbor_export(&eval_out)?,
        });
    }

    let report = RunReport {
        schema_version: RUN_REPORT_SCHEMA,
        output_dir: out.display().to_string(),
        evals,
    };
    write_json(&out.join("run-report.json"), &report)?;
    Ok(report)
}

fn run_code_review_floor(out: &Path) -> anyhow::Result<EvalReport> {
    let artifact = fixture("cerberus-artifact.json");
    let key = fixture("expected-defects.json");
    let (candidates, dropped_invalid) = candidate_rows(&artifact)?;
    let key_rows = load_key_rows(&key)?;
    let result = grade(&candidates, &key_rows);
    let match_rate = MatchRate::from_grade(&result);
    let score = Score::from_match_rate("code_review_recall", &match_rate);

    let evidence = GradeEvidence {
        artifact: artifact.display().to_string(),
        key: key.display().to_string(),
        matched: result.matched.len(),
        disputed: result.disputed.len(),
        missed: result.missed.len(),
        dropped_invalid,
        recoverable_misses: recoverable_misses(&result),
        match_rate,
    };
    let evidence_path = out.join("grade.json");
    write_json(&evidence_path, &evidence)?;

    Ok(EvalReport {
        id: RunEval::CodeReviewDeterministicFloor.id().to_string(),
        title: "Code-review deterministic floor".to_string(),
        score,
        artifacts: vec![evidence_path.display().to_string()],
        notes: vec![
            "Grades the real Cerberus fixture against a Daedalus tests/expected.json scorer key."
                .to_string(),
            "The score is category-strict recall before human adjudication, not final benchmark reward."
                .to_string(),
        ],
    })
}

fn run_recoverable_queue(out: &Path) -> anyhow::Result<EvalReport> {
    let artifact = fixture("cerberus-artifact.json");
    let key = fixture("key-colocated-other-category.json");
    let (candidates, _dropped_invalid) = candidate_rows(&artifact)?;
    let key_rows = load_key_rows(&key)?;
    let result = grade(&candidates, &key_rows);
    let queue = build_queue(&result);
    let recoverable_items = queue
        .items
        .iter()
        .filter(|item| item.is_recoverable())
        .count();
    let score = wilson_score(
        "recoverable_queue_routing",
        recoverable_items as u64,
        queue.items.len() as u64,
    );

    let queue_path = out.join("queue.json");
    write_json(&queue_path, &queue)?;
    let panel_dir = out.join("panel");
    let panel = adjudication_panel::write_panel(&queue_path, &panel_dir)?;

    Ok(EvalReport {
        id: RunEval::RecoverableAdjudicationQueue.id().to_string(),
        title: "Recoverable adjudication queue".to_string(),
        score,
        artifacts: vec![
            queue_path.display().to_string(),
            panel.html_path.display().to_string(),
            panel.queue_path.display().to_string(),
        ],
        notes: vec![
            "Uses a co-located category mismatch to prove the queue routes recoverable misses before plain disputes."
                .to_string(),
            "The panel artifact is the phone-first human surface for this queue."
                .to_string(),
        ],
    })
}

fn run_harbor_export(out: &Path) -> anyhow::Result<EvalReport> {
    let queue_path = fixture("export-queue.json");
    let queue = load_queue(&queue_path)?;
    let ctx = ExportContext {
        arena: "pr-review-v0".to_string(),
        task: "py-file-cache".to_string(),
        date: "2026-07-01".to_string(),
        base_version: "0.2.0"
            .parse::<ArenaVersion>()
            .expect("hard-coded arena version is valid"),
    };
    let adjudications = adjudications_from_queue(&queue, &ctx)?;
    let accepts = adjudications.iter().filter(|a| a.is_accept()).count();
    let expected = extended_expected_key(&ExpectedKey::default(), &adjudications);
    let oracle = crucible_core::extended_key(
        &AnswerKey {
            findings: Vec::new(),
        },
        &adjudications,
    );
    let fully_exported_accepts = adjudications
        .iter()
        .filter(|adjudication| {
            adjudication.is_accept()
                && expected.defects.iter().any(|defect| {
                    defect.file == adjudication.file
                        && defect.category == adjudication.category
                        && (defect.line_start..=defect.line_end).contains(&adjudication.line)
                })
                && oracle.findings.iter().any(|finding| {
                    finding.file == adjudication.file
                        && finding.line == adjudication.line
                        && finding.category == adjudication.category
                })
        })
        .count();
    let score = wilson_score(
        "accepted_findings_exported_to_harbor",
        fully_exported_accepts as u64,
        accepts as u64,
    );

    let adjudications_path = out.join("adjudications.md");
    std::fs::write(
        &adjudications_path,
        render_adjudications_md(&ctx.arena, &adjudications),
    )
    .with_context(|| format!("writing {}", adjudications_path.display()))?;
    let expected_path = out.join("tests").join("expected.json");
    write_json(&expected_path, &expected)?;
    let oracle_path = out.join("solution").join("findings.json");
    write_json(&oracle_path, &oracle)?;

    let evidence_path = out.join("export-evidence.json");
    write_json(
        &evidence_path,
        &ExportEvidence {
            queue: queue_path.display().to_string(),
            adjudications: adjudications.len(),
            accepts,
            fully_exported_accepts,
            expected_defects: expected.defects.len(),
            oracle_findings: oracle.findings.len(),
        },
    )?;

    Ok(EvalReport {
        id: RunEval::HarborExportAcceptance.id().to_string(),
        title: "Harbor export acceptance".to_string(),
        score,
        artifacts: vec![
            adjudications_path.display().to_string(),
            expected_path.display().to_string(),
            oracle_path.display().to_string(),
            evidence_path.display().to_string(),
        ],
        notes: vec![
            "Applies committed labels from the existing export queue fixture and writes the Harbor scorer/oracle artifacts."
                .to_string(),
            "Only Keep + in_scope labels become accepted defects; out-of-scope and noise labels leave the key unchanged."
                .to_string(),
        ],
    })
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(value)
        .with_context(|| format!("serializing {}", path.display()))?;
    std::fs::write(path, format!("{json}\n")).with_context(|| format!("writing {}", path.display()))
}

/// Top-level report for one `crucible run` invocation.
#[derive(Debug, Serialize)]
pub struct RunReport {
    pub schema_version: &'static str,
    pub output_dir: String,
    pub evals: Vec<EvalReport>,
}

/// One concrete eval result.
#[derive(Debug, Serialize)]
pub struct EvalReport {
    pub id: String,
    pub title: String,
    pub score: Score,
    pub artifacts: Vec<String>,
    pub notes: Vec<String>,
}

/// Defensible binary score with a Wilson interval.
#[derive(Debug, Clone, Serialize)]
pub struct Score {
    pub metric: &'static str,
    pub successes: u64,
    pub n: u64,
    pub point: Option<f64>,
    pub lower: f64,
    pub upper: f64,
    pub confidence: f64,
    pub method: &'static str,
}

impl Score {
    fn from_match_rate(metric: &'static str, rate: &MatchRate) -> Self {
        Score {
            metric,
            successes: rate.successes,
            n: rate.n,
            point: rate.point,
            lower: rate.lower,
            upper: rate.upper,
            confidence: rate.confidence,
            method: "Wilson",
        }
    }
}

#[derive(Debug, Serialize)]
struct GradeEvidence {
    artifact: String,
    key: String,
    matched: usize,
    disputed: usize,
    missed: usize,
    dropped_invalid: usize,
    recoverable_misses: usize,
    match_rate: MatchRate,
}

#[derive(Debug, Serialize)]
struct ExportEvidence {
    queue: String,
    adjudications: usize,
    accepts: usize,
    fully_exported_accepts: usize,
    expected_defects: usize,
    oracle_findings: usize,
}

/// Render a score for the human `crucible run` output.
pub fn format_score(score: &Score) -> String {
    match score.point {
        Some(point) => format!(
            "{:.1}%   {:.0}% CI [{:.1}%, {:.1}%]   ({}; {}/{})",
            point * 100.0,
            score.confidence * 100.0,
            score.lower * 100.0,
            score.upper * 100.0,
            score.method,
            score.successes,
            score.n
        ),
        None => format!(
            "n/a   {:.0}% CI [{:.1}%, {:.1}%]   ({}; {}/{})",
            score.confidence * 100.0,
            score.lower * 100.0,
            score.upper * 100.0,
            score.method,
            score.successes,
            score.n
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_root(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("crucible-run-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp root");
        dir
    }

    #[test]
    fn run_all_writes_three_eval_reports_and_artifacts() {
        let out = temp_root("all");
        let report = run(RunEval::All, &out).expect("run built-in evals");
        assert_eq!(report.schema_version, RUN_REPORT_SCHEMA);
        assert_eq!(report.evals.len(), 3);
        assert!(out.join("run-report.json").exists());
        assert!(out
            .join("code-review-deterministic-floor")
            .join("grade.json")
            .exists());
        assert!(out
            .join("recoverable-adjudication-queue")
            .join("panel")
            .join("index.html")
            .exists());
        assert!(out
            .join("harbor-export-acceptance")
            .join("tests")
            .join("expected.json")
            .exists());

        let ids: Vec<_> = report.evals.iter().map(|eval| eval.id.as_str()).collect();
        assert!(ids.contains(&"code-review-deterministic-floor"));
        assert!(ids.contains(&"recoverable-adjudication-queue"));
        assert!(ids.contains(&"harbor-export-acceptance"));
    }

    #[test]
    fn harbor_export_eval_scores_the_single_accept() {
        let out = temp_root("export");
        let report = run(RunEval::HarborExportAcceptance, &out).expect("run export eval");
        let score = &report.evals[0].score;
        assert_eq!(score.metric, "accepted_findings_exported_to_harbor");
        assert_eq!(score.successes, 1);
        assert_eq!(score.n, 1);
        assert_eq!(score.point, Some(1.0));
        assert!(out
            .join("harbor-export-acceptance")
            .join("solution")
            .join("findings.json")
            .exists());
        assert!(
            score.lower < 1.0 && score.upper <= 1.0,
            "Wilson interval remains honest at n=1: {score:?}"
        );
    }
}
