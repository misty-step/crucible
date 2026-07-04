//! Finding records emitted from defensible run comparisons.

use std::path::Path;

use anyhow::{Context, Result};
use crucible_core::{paired_rate_delta_interval, DeltaVerdict};
use serde::Serialize;

use crate::run_store::{ConfigComparison, StoredRun};

pub const FINDINGS_JOURNAL_SCHEMA: &str = "crucible.findings_journal.v1";
pub const FINDING_RECORD_SCHEMA: &str = "crucible.finding.v1";

#[derive(Debug, Serialize)]
pub struct FindingsJournal {
    pub schema_version: &'static str,
    pub source_schema_version: &'static str,
    pub db: String,
    pub benchmark: String,
    pub alpha: f64,
    pub findings: Vec<FindingRecord>,
}

#[derive(Debug, Serialize)]
pub struct FindingRecord {
    pub schema_version: &'static str,
    pub id: String,
    pub kind: &'static str,
    pub hypothesis: String,
    pub benchmark: String,
    pub left: FindingRunRef,
    pub right: FindingRunRef,
    pub stronger: FindingRunRef,
    pub delta: FindingDelta,
    pub paired: FindingPairedOutcome,
    pub chart: FindingChart,
    pub repro_command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindingRunRef {
    pub query: String,
    pub run_id: String,
    pub config_id: String,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FindingDelta {
    pub method: &'static str,
    pub common_tasks: usize,
    pub point: f64,
    pub lower: f64,
    pub upper: f64,
    pub confidence: f64,
}

#[derive(Debug, Serialize)]
pub struct FindingPairedOutcome {
    pub common_tasks: usize,
    pub b: u64,
    pub c: u64,
    pub statistic: f64,
    pub p_value: f64,
    pub alpha: f64,
    pub verdict: DeltaVerdict,
}

#[derive(Debug, Serialize)]
pub struct FindingChart {
    pub kind: &'static str,
    pub x_axis: &'static str,
    pub delta: FindingChartInterval,
    pub left_only_successes: FindingDiscordantRate,
    pub right_only_successes: FindingDiscordantRate,
}

#[derive(Debug, Serialize)]
pub struct FindingChartInterval {
    pub label: &'static str,
    pub point: f64,
    pub lower: f64,
    pub upper: f64,
}

#[derive(Debug, Serialize)]
pub struct FindingDiscordantRate {
    pub label: &'static str,
    pub count: u64,
    pub common_tasks: usize,
    pub point: f64,
}

pub fn journal_from_comparison(
    comparison: &ConfigComparison,
    alpha: f64,
    repro_command: String,
) -> FindingsJournal {
    let findings = finding_from_comparison(comparison, alpha, repro_command)
        .into_iter()
        .collect();
    FindingsJournal {
        schema_version: FINDINGS_JOURNAL_SCHEMA,
        source_schema_version: comparison.schema_version,
        db: comparison.db.clone(),
        benchmark: comparison.benchmark.clone(),
        alpha,
        findings,
    }
}

pub fn write_journal(
    comparison: &ConfigComparison,
    alpha: f64,
    repro_command: String,
    path: &Path,
) -> Result<FindingsJournal> {
    let journal = journal_from_comparison(comparison, alpha, repro_command);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating findings journal dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&journal).context("serializing findings journal")?;
    std::fs::write(path, format!("{json}\n"))
        .with_context(|| format!("writing findings journal {}", path.display()))?;
    Ok(journal)
}

fn finding_from_comparison(
    comparison: &ConfigComparison,
    alpha: f64,
    repro_command: String,
) -> Option<FindingRecord> {
    let paired = comparison.paired.as_ref()?;
    if !paired.verdict.is_signal() {
        return None;
    }

    if comparison.common_tasks == 0 {
        return None;
    }
    let confidence = confidence_from_alpha(alpha);
    let interval =
        paired_rate_delta_interval(paired.b, paired.c, comparison.common_tasks, confidence);
    let delta = interval.point;
    let left = run_ref(&comparison.left_query, &comparison.left);
    let right = run_ref(&comparison.right_query, &comparison.right);
    let stronger = if delta >= 0.0 {
        right.clone()
    } else {
        left.clone()
    };
    let weaker = if delta >= 0.0 { &left } else { &right };
    let magnitude = delta.abs();

    Some(FindingRecord {
        schema_version: FINDING_RECORD_SCHEMA,
        id: finding_id(comparison, &left.run_id, &right.run_id),
        kind: "eval_delta",
        hypothesis: format!(
            "{} outperforms {} on {} by {:.4}",
            display_run(&stronger),
            display_run(weaker),
            comparison.benchmark,
            magnitude
        ),
        benchmark: comparison.benchmark.clone(),
        left: left.clone(),
        right: right.clone(),
        stronger,
        delta: FindingDelta {
            method: "paired_shared_task_rate_delta",
            common_tasks: comparison.common_tasks,
            point: interval.point,
            lower: interval.lower,
            upper: interval.upper,
            confidence: interval.confidence,
        },
        paired: FindingPairedOutcome {
            common_tasks: comparison.common_tasks,
            b: paired.b,
            c: paired.c,
            statistic: paired.statistic,
            p_value: paired.p_value,
            alpha,
            verdict: paired.verdict,
        },
        chart: FindingChart {
            kind: "paired_delta_interval",
            x_axis: "right_minus_left_shared_task_pass_rate",
            delta: FindingChartInterval {
                label: "paired shared-task delta",
                point: interval.point,
                lower: interval.lower,
                upper: interval.upper,
            },
            left_only_successes: FindingDiscordantRate {
                label: "left passed, right failed",
                count: paired.b,
                common_tasks: comparison.common_tasks,
                point: paired.b as f64 / comparison.common_tasks as f64,
            },
            right_only_successes: FindingDiscordantRate {
                label: "right passed, left failed",
                count: paired.c,
                common_tasks: comparison.common_tasks,
                point: paired.c as f64 / comparison.common_tasks as f64,
            },
        },
        repro_command,
    })
}

fn run_ref(query: &str, run: &StoredRun) -> FindingRunRef {
    FindingRunRef {
        query: query.to_string(),
        run_id: run.run_id.clone(),
        config_id: run.config_id.clone(),
        model: run.model.clone(),
    }
}

fn display_run(run: &FindingRunRef) -> String {
    run.model
        .as_deref()
        .unwrap_or(run.config_id.as_str())
        .to_string()
}

fn finding_id(comparison: &ConfigComparison, left_run_id: &str, right_run_id: &str) -> String {
    format!(
        "finding-{}-{}-{}",
        slug(&comparison.benchmark),
        slug(left_run_id),
        slug(right_run_id)
    )
}

fn slug(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn confidence_from_alpha(alpha: f64) -> f64 {
    if alpha.is_finite() && alpha > 0.0 && alpha < 1.0 {
        1.0 - alpha
    } else {
        0.95
    }
}

#[cfg(test)]
mod tests {
    use crucible_core::McnemarOutcome;

    use super::*;
    use crate::run_store::{ClassComparison, ConfigComparison};

    #[test]
    fn emits_a_finding_only_for_a_paired_signal() {
        let comparison = comparison_with_verdict(DeltaVerdict::Signal);
        let journal = journal_from_comparison(
            &comparison,
            0.05,
            "crucible runs compare --json".to_string(),
        );

        assert_eq!(journal.schema_version, FINDINGS_JOURNAL_SCHEMA);
        assert_eq!(journal.findings.len(), 1);
        let finding = &journal.findings[0];
        assert_eq!(finding.schema_version, FINDING_RECORD_SCHEMA);
        assert_eq!(finding.kind, "eval_delta");
        assert_eq!(finding.benchmark, "prompt-smoke-v0");
        assert_eq!(finding.stronger.query, "test/model-b");
        assert_eq!(finding.delta.method, "paired_shared_task_rate_delta");
        assert_eq!(finding.delta.common_tasks, 24);
        assert_eq!(finding.delta.point, 14.0 / 24.0);
        assert_eq!(finding.delta.confidence, 0.95);
        assert!(finding.delta.lower > 0.0);
        assert!(finding.delta.upper > finding.delta.point);
        assert_eq!(finding.paired.common_tasks, 24);
        assert_eq!(finding.paired.verdict, DeltaVerdict::Signal);
        assert_eq!(finding.chart.kind, "paired_delta_interval");
        assert_eq!(finding.chart.delta.point, finding.delta.point);
        assert_eq!(finding.chart.left_only_successes.point, 1.0 / 24.0);
        assert_eq!(finding.chart.right_only_successes.point, 15.0 / 24.0);
        assert!(finding
            .hypothesis
            .contains("test/model-b outperforms test/model-a"));
        assert_eq!(finding.repro_command, "crucible runs compare --json");
    }

    #[test]
    fn finding_direction_comes_from_the_paired_population_not_whole_run_points() {
        let mut comparison = comparison_with_verdict(DeltaVerdict::Signal);
        comparison.left.point = Some(0.9);
        comparison.right.point = Some(0.8);
        comparison.delta_point = Some(-0.1);
        comparison.paired = Some(McnemarOutcome {
            b: 1,
            c: 15,
            statistic: 10.5625,
            p_value: 0.001,
            verdict: DeltaVerdict::Signal,
        });

        let journal = journal_from_comparison(
            &comparison,
            0.05,
            "crucible runs compare --json".to_string(),
        );

        let finding = &journal.findings[0];
        assert_eq!(finding.stronger.query, "test/model-b");
        assert_eq!(finding.delta.point, 14.0 / 24.0);
        assert_eq!(finding.chart.delta.point, 14.0 / 24.0);
        assert_eq!(finding.chart.left_only_successes.count, 1);
        assert_eq!(finding.chart.right_only_successes.count, 15);
        assert!(
            finding
                .hypothesis
                .contains("test/model-b outperforms test/model-a"),
            "hypothesis must follow the paired signal, not the whole-run point delta: {}",
            finding.hypothesis
        );
    }

    #[test]
    fn refuses_to_emit_a_finding_inside_the_noise_floor() {
        let comparison = comparison_with_verdict(DeltaVerdict::InsideNoiseFloor);
        let journal = journal_from_comparison(
            &comparison,
            0.05,
            "crucible runs compare --json".to_string(),
        );

        assert!(journal.findings.is_empty());
    }

    #[test]
    fn writes_a_pretty_json_journal_file() {
        let dir =
            std::env::temp_dir().join(format!("crucible-findings-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = dir.join("nested/findings.json");
        let comparison = comparison_with_verdict(DeltaVerdict::Signal);

        let journal = write_journal(
            &comparison,
            0.05,
            "crucible runs compare --json".to_string(),
            &out,
        )
        .expect("write findings journal");
        assert_eq!(journal.findings.len(), 1);

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).expect("read journal"))
                .expect("journal is JSON");
        assert_eq!(written["schema_version"], FINDINGS_JOURNAL_SCHEMA);
        assert_eq!(written["findings"][0]["paired"]["verdict"], "signal");
        assert_eq!(
            written["findings"][0]["chart"]["kind"],
            "paired_delta_interval"
        );
        assert!(written["findings"][0]["left"].get("point").is_none());
        assert!(written["findings"][0]["right"].get("lower").is_none());
    }

    fn comparison_with_verdict(verdict: DeltaVerdict) -> ConfigComparison {
        ConfigComparison {
            schema_version: "crucible.run_store.v1",
            db: "runs/local/test.sqlite".to_string(),
            benchmark: "prompt-smoke-v0".to_string(),
            left_query: "test/model-a".to_string(),
            right_query: "test/model-b".to_string(),
            left: stored_run("left-run", "test/model-a", 2, 10, 0.2, 0.1, 0.4),
            right: stored_run("right-run", "test/model-b", 8, 10, 0.8, 0.6, 0.9),
            delta_point: Some(0.6),
            common_tasks: 24,
            paired: Some(McnemarOutcome {
                b: 1,
                c: 15,
                statistic: 10.5625,
                p_value: 0.001,
                verdict,
            }),
            class_breakdowns: Vec::<ClassComparison>::new(),
            comparison_kind: "paired_mcnemar",
            note: "Paired McNemar comparison over prompt tasks common to both runs; see paired.verdict for the noise-floor decision.",
        }
    }

    fn stored_run(
        run_id: &str,
        model: &str,
        successes: u64,
        n: u64,
        point: f64,
        lower: f64,
        upper: f64,
    ) -> StoredRun {
        StoredRun {
            run_id: run_id.to_string(),
            invocation_id: format!("invocation-{run_id}"),
            benchmark_id: "prompt-smoke-v0".to_string(),
            title: "Prompt smoke".to_string(),
            runner_kind: "prompt_benchmark".to_string(),
            config_id: model.to_string(),
            provider: Some("open_router".to_string()),
            model: Some(model.to_string()),
            created_at_unix_ms: 1,
            output_dir: "runs/local/test".to_string(),
            run_report: "runs/local/test/run-report.json".to_string(),
            evidence_path: Some("runs/local/test/prompt-run.json".to_string()),
            spec_path: Some("evals/prompt-smoke-v0.json".to_string()),
            score_metric: "prompt_rubric_pass_rate".to_string(),
            successes,
            n,
            point: Some(point),
            lower,
            upper,
            confidence: 0.95,
            method: "Wilson".to_string(),
        }
    }
}
