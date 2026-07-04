//! Render an ingested [`Dataset`] + computed [`Leaderboard`] into one
//! self-contained, phone-first HTML dashboard — no server, no external asset.
//!
//! This is the *view* layer of the read side: [`crucible_core::dashboard`] turns
//! a tree of real Daedalus arenas and runs into a typed model and measures it;
//! this module turns that model into a single `index.html` a phone can open from
//! the filesystem. The statistics are never recomputed here — every interval,
//! rank, and noise-floor verdict is read straight off the [`Leaderboard`], so the
//! page can only *display* the measured truth, never invent a new one.
//!
//! # The model the page renders
//!
//! [`DashboardData`] is the whole render input and the exact shape written to
//! `data.json` beside the HTML (one stable, inspectable artifact per number):
//! the borrowed [`Dataset`] and [`Leaderboard`], the source directories for
//! provenance, and the [`RunDetail`]s that drive the run drill-down. The first
//! three views below are pure projections of the [`Leaderboard`]/[`Dataset`]; the
//! drill-down needs a slice neither preserves — the per-**directory** trial grid —
//! so [`run_details`] re-walks the runs tree (reusing
//! [`crucible_core::Trial`] for the parse) to recover it. That is a deliberately
//! different projection of the same files: the [`Dataset`] pools a config's trials
//! across every run that produced its `composition_hash` and discards which
//! directory each came from; the drill-down keeps exactly that discarded
//! dimension. Both honor the same trap — a run directory's name can lie about its
//! arena, so the arena is always taken from the trials, never the directory name.
//!
//! # Three views, one scroll
//!
//! 1. **Evals** — every `(arena_id, arena_version)` group: tasks, runs, and the
//!    top config's reward, each row jumping to its detail.
//! 2. **Eval detail** — per group, the **leaderboard** as the centerpiece: each
//!    config's `reward_mean` as a bar with bootstrap-CI whiskers, its `solve_rate`
//!    with a Wilson CI, the sample sizes, and a noise-floor badge on the verdict
//!    against the next-ranked config (`≫ stronger than runner-up` when the gap
//!    clears the floor, `≈ inside noise floor` when it does not). Each interval
//!    method is labeled where it is shown.
//! 3. **Runs** — per run directory, a per-task grid of `reward / recall / fp` for
//!    every config it exercised.
//!
//! # Honesty by construction
//!
//! Provenance is everywhere: every group names the runs feeding it, every config
//! shows its `composition_hash`, and `arena_version` is rendered prominently on
//! every surface. A comparison is **only ever within one group** — the page never
//! places two arena versions side by side, because their scoring keys differ and
//! the rewards are not comparable. Sample sizes (`n_trials`, `n_tasks`,
//! `n_errors`) are always shown, and a wide interval on a tiny `n` is displayed at
//! its true width, never hidden: a single-task config's collapsed CI is labeled as
//! such rather than dressed up as precision.
//!
//! The page degrades without JavaScript: the tiny inlined script turns the three
//! sections into tabs, but with scripting off every section is simply visible in
//! one scroll — still fully navigable on a phone with no server.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crucible_core::{
    Dataset, DeltaSign, Estimate, IntervalMethod, Leaderboard, LeaderboardEntry, LeaderboardGroup,
    PairwiseVerdict, Run, Stronger, Trial,
};

/// Stable schema id stamped on the emitted `data.json`, so a downstream reader can
/// pin the artifact shape the same way the other Crucible artifacts are pinned.
pub const DASHBOARD_SCHEMA: &str = "crucible.dashboard.v1";

/// The whole render input, and the exact object serialized to `data.json`.
///
/// Borrows the [`Dataset`] and [`Leaderboard`] (the page never mutates or
/// recomputes them) plus the [`RunDetail`] slice that drives the drill-down, and
/// carries the source directories so every number on the page traces back to the
/// tree it came from.
#[derive(Debug, Serialize)]
pub struct DashboardData<'a> {
    /// Always [`DASHBOARD_SCHEMA`]; first field so it leads the emitted object.
    pub schema_version: &'static str,
    /// Absolute path of the arenas tree the answer keys were read from.
    pub arenas_dir: String,
    /// Absolute path of the runs tree the trials were read from.
    pub runs_dir: String,
    /// The ingested corpus: groups, run receipts, and the skip count.
    pub dataset: &'a Dataset,
    /// The measured ranking computed from [`dataset`](Self::dataset).
    pub leaderboard: &'a Leaderboard,
    /// Per-directory trial grids for the run drill-down (see [`run_details`]).
    pub run_details: &'a [RunDetail],
}

/// One run **directory**, projected for the drill-down: which arena its trials
/// claim and a per-task cell for every config it exercised.
///
/// The directory-preserving counterpart to a [`Run`] receipt: where [`Run`] keeps
/// only counts, this keeps the actual per-`(config, task)` outcomes the grid
/// needs. [`arena_id`](Self::arena_id)/[`arena_version`](Self::arena_version) are
/// the dominant arena among the directory's placeable trials — the trials' claim,
/// never the directory name, which is surfaced separately as
/// [`dir`](Self::dir) so the discrepancy stays visible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunDetail {
    /// Directory name on disk (the human label, which may disagree with the arena).
    pub dir: String,
    /// Arena id its trials claim — the truth, read from `trials.jsonl`.
    pub arena_id: String,
    /// Arena version its trials claim.
    pub arena_version: String,
    /// Distinct tasks the directory exercised, sorted.
    pub tasks: Vec<String>,
    /// Configs the directory exercised, sorted by `composition_hash`.
    pub configs: Vec<RunConfigCells>,
}

/// One config's per-task cells within a single run directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConfigCells {
    /// The config's stable identity.
    pub composition_hash: String,
    /// Display label (the first trial's `candidate_id`).
    pub id: String,
    /// Display kind (the first trial's `candidate_kind`).
    pub kind: String,
    /// One cell per task this config ran in the directory, sorted by task id.
    pub cells: Vec<RunCell>,
}

/// One `(config, task)` cell in a run directory: the mean outcomes over the
/// (usually few) trials that config ran on that task in that directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunCell {
    /// Task id this cell scores.
    pub task_id: String,
    /// Mean reward over the cell's trials (continuous, `0..=1`).
    pub reward: f64,
    /// Mean recall over the cell's trials.
    pub recall: f64,
    /// Mean false-positive count per trial in the cell.
    pub false_positives: f64,
    /// Trials behind the cell.
    pub n_trials: usize,
    /// How many of those trials were error trials (harness/scorer aborts).
    pub n_errors: usize,
}

/// Accumulator for one `(config, task)` cell while walking a directory.
#[derive(Default)]
struct CellAcc {
    reward_sum: f64,
    recall_sum: f64,
    fp_sum: i64,
    n: usize,
    errors: usize,
}

/// Walk `runs_dir` and build one [`RunDetail`] per directory for the drill-down.
///
/// Total, like [`Dataset::load`]: an unreadable `runs_dir` yields an empty vec, a
/// directory with no readable `trials.jsonl` contributes nothing, and a line that
/// fails to parse or lacks arena/hash identity is skipped. Directories and their
/// contents are emitted in sorted order so the artifact is deterministic. The
/// arena label per directory is the dominant `(arena_id, arena_version)` among its
/// placeable trials — read from the trials, never the directory name.
pub fn run_details(runs_dir: impl AsRef<Path>) -> Vec<RunDetail> {
    let runs_dir = runs_dir.as_ref();
    let mut entries: Vec<_> = match std::fs::read_dir(runs_dir) {
        Ok(read_dir) => read_dir.flatten().collect(),
        Err(_) => return Vec::new(),
    };
    entries.sort_by_key(std::fs::DirEntry::file_name);

    let mut details = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path.join("trials.jsonl")) else {
            continue;
        };
        if let Some(detail) = run_detail(entry.file_name().to_string_lossy().into_owned(), &content)
        {
            details.push(detail);
        }
    }
    details
}

/// Build one [`RunDetail`] from a directory's name and `trials.jsonl` text, or
/// `None` when the directory yielded no placeable trial.
fn run_detail(dir: String, content: &str) -> Option<RunDetail> {
    // Per-config display labels, per-config-per-task accumulators, the set of
    // tasks, and the (arena_id, arena_version) vote — all keyed off the trial.
    let mut labels: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut cells: BTreeMap<(String, String), CellAcc> = BTreeMap::new();
    let mut tasks: BTreeSet<String> = BTreeSet::new();
    let mut arena_votes: BTreeMap<(String, String), usize> = BTreeMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(trial) = serde_json::from_str::<Trial>(line) else {
            continue;
        };
        if trial.arena_id.is_empty()
            || trial.arena_version.is_empty()
            || trial.composition_hash.is_empty()
        {
            continue;
        }

        *arena_votes
            .entry((trial.arena_id.clone(), trial.arena_version.clone()))
            .or_default() += 1;
        labels
            .entry(trial.composition_hash.clone())
            .or_insert_with(|| (trial.candidate_id.clone(), trial.candidate_kind.clone()));
        tasks.insert(trial.task_id.clone());

        let acc = cells
            .entry((trial.composition_hash.clone(), trial.task_id.clone()))
            .or_default();
        acc.reward_sum += trial.reward;
        acc.recall_sum += trial.recall;
        acc.fp_sum += trial.false_positives;
        acc.n += 1;
        if trial.is_error() {
            acc.errors += 1;
        }
    }

    // The dominant arena (ties to the largest key, deterministically); `None` only
    // when the directory had no placeable trial at all.
    let (arena_id, arena_version) = arena_votes
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
        .map(|(key, _)| key)?;

    let configs = labels
        .into_iter()
        .map(|(hash, (id, kind))| {
            let cells = tasks
                .iter()
                .filter_map(|task| {
                    cells.get(&(hash.clone(), task.clone())).map(|acc| {
                        let n = acc.n.max(1) as f64;
                        RunCell {
                            task_id: task.clone(),
                            reward: acc.reward_sum / n,
                            recall: acc.recall_sum / n,
                            false_positives: acc.fp_sum as f64 / n,
                            n_trials: acc.n,
                            n_errors: acc.errors,
                        }
                    })
                })
                .collect();
            RunConfigCells {
                composition_hash: hash,
                id,
                kind,
                cells,
            }
        })
        .collect();

    Some(RunDetail {
        dir,
        arena_id,
        arena_version,
        tasks: tasks.into_iter().collect(),
        configs,
    })
}

/// Render the whole dashboard to one self-contained HTML document.
pub fn render(data: &DashboardData<'_>) -> String {
    let dataset = data.dataset;
    let board = data.leaderboard;

    // Index the leaderboard group and the run receipts by (arena_id, version) so
    // each eval detail can pull its ranking and its provenance in one lookup.
    let groups_by_key: BTreeMap<(&str, &str), &LeaderboardGroup> = board
        .groups
        .iter()
        .map(|g| ((g.arena_id.as_str(), g.arena_version.as_str()), g))
        .collect();
    let mut runs_by_key: BTreeMap<(&str, &str), Vec<&Run>> = BTreeMap::new();
    for run in &dataset.runs {
        runs_by_key
            .entry((run.arena_id.as_str(), run.arena_version.as_str()))
            .or_default()
            .push(run);
    }

    let mut out = String::with_capacity(64 * 1024);
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<title>Crucible — eval dashboard</title>\n");
    out.push_str(r#"<link rel="icon" type="image/svg+xml" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%231a1a1a' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2'/%3E%3Cpath d='M6.453 15h11.094'/%3E%3Cpath d='M8.5 2h7'/%3E%3C/svg%3E">"#);
    out.push_str("\n<style>\n");
    out.push_str(STYLE);
    out.push_str("\n</style>\n</head>\n<body>\n");

    render_header(&mut out, data);
    render_nav(&mut out);

    out.push_str("<main>\n");
    render_evals_view(&mut out, dataset, &groups_by_key, &runs_by_key);
    render_detail_view(&mut out, dataset, &groups_by_key, &runs_by_key);
    render_runs_view(&mut out, data.run_details);
    render_legend(&mut out, board);
    out.push_str("</main>\n");

    out.push_str("<script>\n");
    out.push_str(SCRIPT);
    out.push_str("\n</script>\n</body>\n</html>\n");
    out
}

/// The page header: title plus the provenance line (source trees and corpus
/// totals) that grounds every number below.
fn render_header(out: &mut String, data: &DashboardData<'_>) {
    let ds = data.dataset;
    let board = data.leaderboard;
    out.push_str("<header class=\"top\">\n");
    out.push_str(concat!(
        "<h1><svg class=\"ae-icon\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" ",
        "stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">",
        "<path d=\"M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96",
        "l5.51-10.08A2 2 0 0 0 10 8V2\"/><path d=\"M6.453 15h11.094\"/><path d=\"M8.5 2h7\"/></svg>",
        "Crucible<span class=\"sub\">eval dashboard</span></h1>\n"
    ));
    out.push_str("<div class=\"prov\">\n");
    out.push_str(&format!(
        "<span><b>{}</b> evals</span><span><b>{}</b> runs</span><span><b>{}</b> trials</span>",
        ds.group_count(),
        ds.runs.len(),
        ds.trial_count(),
    ));
    if ds.skipped > 0 {
        out.push_str(&format!(
            "<span class=\"warn\"><b>{}</b> skipped</span>",
            ds.skipped
        ));
    }
    if !ds.skipped_inputs.is_empty() {
        out.push_str(&format!(
            "<span class=\"warn\"><b>{}</b> skipped inputs</span>",
            ds.skipped_inputs.len()
        ));
    }
    out.push_str(&format!(
        "<span class=\"ci\">CI {:.0}% · {} resamples · seed {:#018x}</span>",
        board.confidence * 100.0,
        board.resamples,
        board.seed,
    ));
    out.push_str("</div>\n");
    out.push_str(&format!(
        "<div class=\"paths\">arenas <code>{}</code> · runs <code>{}</code></div>\n",
        escape_html(&data.arenas_dir),
        escape_html(&data.runs_dir),
    ));
    out.push_str("</header>\n");
}

/// The sticky three-tab navigation.
fn render_nav(out: &mut String) {
    out.push_str("<nav class=\"tabs\">\n");
    out.push_str("<a href=\"#view-evals\" data-view=\"view-evals\" class=\"active\">Evals</a>\n");
    out.push_str("<a href=\"#view-detail\" data-view=\"view-detail\">Eval detail</a>\n");
    out.push_str("<a href=\"#view-runs\" data-view=\"view-runs\">Runs</a>\n");
    out.push_str("</nav>\n");
}

/// View 1 — the evals overview: one row per group, jumping to its detail.
fn render_evals_view(
    out: &mut String,
    dataset: &Dataset,
    groups: &BTreeMap<(&str, &str), &LeaderboardGroup>,
    runs: &BTreeMap<(&str, &str), Vec<&Run>>,
) {
    out.push_str("<section id=\"view-evals\" class=\"view active\">\n");
    out.push_str("<h2>Evals</h2>\n");
    out.push_str(
        "<p class=\"note\">Every <code>(arena, version)</code> group. A reward is only \
         comparable within one version — versions are never pooled.</p>\n",
    );

    if dataset.evals.is_empty() {
        out.push_str("<p class=\"empty\">No evals ingested.</p>\n</section>\n");
        return;
    }

    out.push_str("<div class=\"cards\">\n");
    for eval in &dataset.evals {
        let key = (eval.arena_id.as_str(), eval.arena_version.as_str());
        let slug = slug(&eval.arena_id, &eval.arena_version);
        let n_runs = runs.get(&key).map_or(0, Vec::len);
        let top = groups.get(&key).and_then(|g| g.entries.first());

        out.push_str(&format!(
            "<a class=\"card\" data-jump=\"{slug}\" href=\"#{slug}\">\n"
        ));
        out.push_str(&format!(
            "<div class=\"card-h\"><span class=\"arena\">{}</span>\
             <span class=\"ver\">{}</span></div>\n",
            escape_html(&eval.arena_id),
            escape_html(&eval.arena_version),
        ));
        out.push_str(&format!(
            "<div class=\"card-m\"><span>{} task{}</span><span>{} run{}</span>\
             <span>{} config{}</span></div>\n",
            eval.tasks.len(),
            plural(eval.tasks.len()),
            n_runs,
            plural(n_runs),
            eval.configs.len(),
            plural(eval.configs.len()),
        ));
        match top {
            Some(entry) => out.push_str(&format!(
                "<div class=\"card-top\"><span class=\"lead\">#1 {}</span>\
                 <span class=\"rew\">{}</span></div>\n",
                escape_html(&entry.id),
                fmt2(entry.reward_mean.point),
            )),
            None => out.push_str("<div class=\"card-top muted\">no ranked config</div>\n"),
        }
        out.push_str("</a>\n");
    }
    out.push_str("</div>\n</section>\n");
}

/// View 2 — per-group detail with the leaderboard as the centerpiece.
fn render_detail_view(
    out: &mut String,
    dataset: &Dataset,
    groups: &BTreeMap<(&str, &str), &LeaderboardGroup>,
    runs: &BTreeMap<(&str, &str), Vec<&Run>>,
) {
    out.push_str("<section id=\"view-detail\" class=\"view\">\n");
    out.push_str("<h2>Eval detail</h2>\n");

    if dataset.evals.is_empty() {
        out.push_str("<p class=\"empty\">No evals ingested.</p>\n</section>\n");
        return;
    }

    for eval in &dataset.evals {
        let key = (eval.arena_id.as_str(), eval.arena_version.as_str());
        let slug = slug(&eval.arena_id, &eval.arena_version);

        out.push_str(&format!("<section class=\"eval\" id=\"{slug}\">\n"));
        out.push_str(&format!(
            "<div class=\"eval-h\"><span class=\"arena\">{}</span>\
             <span class=\"ver big\">{}</span></div>\n",
            escape_html(&eval.arena_id),
            escape_html(&eval.arena_version),
        ));

        // Tasks: id + how many defects the key seeded.
        out.push_str("<div class=\"block\"><h3>Tasks</h3><div class=\"chips\">\n");
        if eval.tasks.is_empty() {
            out.push_str("<span class=\"muted\">none</span>");
        }
        for task in &eval.tasks {
            out.push_str(&format!(
                "<span class=\"chip\">{}<small>{} defect{}</small></span>\n",
                escape_html(&task.id),
                task.defects.len(),
                plural(task.defects.len()),
            ));
        }
        out.push_str("</div></div>\n");

        // Runs feeding this group — provenance for every number in the board.
        out.push_str(
            "<div class=\"block\"><h3>Runs feeding this eval</h3><div class=\"runlist\">\n",
        );
        match runs.get(&key) {
            Some(rs) if !rs.is_empty() => {
                for run in rs {
                    let mismatch = !run.dir.contains(&run.arena_id);
                    out.push_str(&format!(
                        "<div class=\"runref\"><code>{}</code><span>{} trial{}</span>{}</div>\n",
                        escape_html(&run.dir),
                        run.trial_count,
                        plural(run.trial_count),
                        if mismatch {
                            "<span class=\"flag\" title=\"directory name disagrees with the \
                             arena its trials claim\">name≠arena</span>"
                        } else {
                            ""
                        },
                    ));
                }
            }
            _ => out.push_str("<span class=\"muted\">none</span>"),
        }
        out.push_str("</div></div>\n");

        // The leaderboard.
        match groups.get(&key) {
            Some(group) if !group.entries.is_empty() => render_leaderboard(out, group),
            _ => out.push_str("<p class=\"empty\">No ranked configs in this group.</p>\n"),
        }

        out.push_str("</section>\n");
    }
    out.push_str("</section>\n");
}

/// The leaderboard table for one group: the centerpiece of the detail view.
fn render_leaderboard(out: &mut String, group: &LeaderboardGroup) {
    out.push_str("<div class=\"block\"><h3>Leaderboard</h3>\n");
    out.push_str(
        "<p class=\"note\">Ranked by mean reward. The bar is the mean with its \
         <b>95% bootstrap CI</b> (resampled over tasks); solve rate carries a \
         <b>Wilson CI</b> (task-level: fraction of tasks fully solved, n = tasks). \
         The badge is the seed-stable, directional noise-floor verdict against the \
         next config — a <b>signal</b> names which config is stronger, or the gap \
         is <b>inside the noise floor</b> or <b>underpowered</b> (too few shared \
         tasks to test).</p>\n",
    );
    out.push_str("<ol class=\"board\">\n");
    for entry in &group.entries {
        render_entry(out, entry);
    }
    out.push_str("</ol>\n</div>\n");
}

/// One leaderboard row: identity, the reward bar with CI whiskers, the solve rate
/// with its Wilson CI, the sample sizes, and the noise-floor badge.
fn render_entry(out: &mut String, entry: &LeaderboardEntry) {
    out.push_str("<li class=\"row\">\n");

    // Identity line.
    out.push_str(&format!(
        "<div class=\"row-h\"><span class=\"rank\">#{}</span>\
         <span class=\"cfg\">{}</span><span class=\"kind\">{}</span>\
         <code class=\"hash\">{}</code></div>\n",
        entry.rank,
        escape_html(&entry.id),
        escape_html(&entry.kind),
        escape_html(&entry.composition_hash),
    ));

    // Reward mean bar + bootstrap CI whiskers.
    out.push_str("<div class=\"metric\">\n");
    out.push_str("<div class=\"metric-l\">reward<small>bootstrap CI</small></div>\n");
    render_bar(out, &entry.reward_mean);
    out.push_str(&format!(
        "<div class=\"metric-v\">{}<small>[{}, {}]</small></div>\n",
        fmt2(entry.reward_mean.point),
        fmt2(entry.reward_mean.lower),
        fmt2(entry.reward_mean.upper),
    ));
    out.push_str("</div>\n");

    // Solve rate + Wilson CI.
    out.push_str("<div class=\"metric\">\n");
    out.push_str("<div class=\"metric-l\">solve<small>Wilson CI</small></div>\n");
    render_bar(out, &entry.solve_rate);
    out.push_str(&format!(
        "<div class=\"metric-v\">{}<small>[{}, {}]</small></div>\n",
        pct(entry.solve_rate.point),
        pct(entry.solve_rate.lower),
        pct(entry.solve_rate.upper),
    ));
    out.push_str("</div>\n");

    // Sample sizes — shown, never hidden; a tiny n is owned, not dressed up.
    let collapsed = entry.n_tasks <= 1;
    out.push_str(&format!(
        "<div class=\"n\">n={} trials · {} task{}{}{}</div>\n",
        entry.n_trials,
        entry.n_tasks,
        plural(entry.n_tasks),
        if entry.n_errors > 0 {
            format!(" · {} error{}", entry.n_errors, plural(entry.n_errors))
        } else {
            String::new()
        },
        if collapsed {
            " · single task — CI collapsed"
        } else {
            ""
        },
    ));

    render_verdict(out, entry);
    out.push_str("</li>\n");
}

/// The noise-floor badge for a row's comparison to the next-ranked config.
fn render_verdict(out: &mut String, entry: &LeaderboardEntry) {
    let Some(vs) = &entry.vs_next else {
        out.push_str("<div class=\"verdict base\">baseline — nothing ranked below</div>\n");
        return;
    };
    let mc = &vs.mcnemar;
    let rd = &vs.reward_delta;
    let tasks = format!(
        "{} shared task{}",
        vs.n_shared_tasks,
        plural(vs.n_shared_tasks)
    );
    match vs.verdict {
        PairwiseVerdict::Signal { stronger } => {
            // Name which test carried it, so the badge is auditable.
            let mut carriers = Vec::new();
            if mc.verdict.is_signal() {
                carriers.push(format!("McNemar p={}", fmt_p(mc.p_value)));
            }
            if rd.sign != DeltaSign::Zero {
                carriers.push(format!(
                    "Δreward {} [{}, {}] paired bootstrap",
                    signed(rd.delta),
                    fmt2(rd.lower),
                    fmt2(rd.upper),
                ));
            }
            // The direction is read off the verdict, never assumed: a #1 whose lead
            // lives only on non-shared tasks correctly shows the runner-up ahead.
            let headline = match stronger {
                Stronger::Higher => "≫ stronger than runner-up",
                Stronger::RunnerUp => "≪ runner-up is stronger here",
            };
            out.push_str(&format!(
                "<div class=\"verdict signal\">{headline}\
                 <small>{} · {tasks}</small></div>\n",
                carriers.join(" · "),
            ));
        }
        PairwiseVerdict::InsideNoiseFloor => {
            out.push_str(&format!(
                "<div class=\"verdict noise\">≈ inside noise floor\
                 <small>McNemar p={} · Δreward {} [{}, {}] · {tasks}</small></div>\n",
                fmt_p(mc.p_value),
                signed(rd.delta),
                fmt2(rd.lower),
                fmt2(rd.upper),
            ));
        }
        PairwiseVerdict::Underpowered => {
            out.push_str(&format!(
                "<div class=\"verdict thin\">? underpowered — too few shared tasks\
                 <small>{tasks} · need ≥6 to test</small></div>\n",
            ));
        }
    }
}

/// A horizontal `0..=1` bar: a fill to the point estimate, the CI as a band with
/// end-tick whiskers, and a point marker. All positions are clamped into the
/// track, and a collapsed interval simply lands every mark at the point.
fn render_bar(out: &mut String, est: &Estimate) {
    let pt = clamp01(est.point) * 100.0;
    let lo = clamp01(est.lower) * 100.0;
    let hi = clamp01(est.upper) * 100.0;
    let method = match est.method {
        IntervalMethod::Bootstrap => "bootstrap",
        IntervalMethod::Wilson => "wilson",
    };
    out.push_str(&format!("<div class=\"bar {method}\">\n"));
    out.push_str(&format!(
        "<div class=\"fill\" style=\"width:{pt:.2}%\"></div>\n"
    ));
    out.push_str(&format!(
        "<div class=\"ciband\" style=\"left:{lo:.2}%;width:{:.2}%\"></div>\n",
        (hi - lo).max(0.0),
    ));
    out.push_str(&format!(
        "<i class=\"whisk\" style=\"left:{lo:.2}%\"></i>\n"
    ));
    out.push_str(&format!(
        "<i class=\"whisk\" style=\"left:{hi:.2}%\"></i>\n"
    ));
    out.push_str(&format!("<i class=\"pt\" style=\"left:{pt:.2}%\"></i>\n"));
    out.push_str("</div>\n");
}

/// View 3 — per run directory, a per-task grid of `reward / recall / fp` per
/// config.
fn render_runs_view(out: &mut String, runs: &[RunDetail]) {
    out.push_str("<section id=\"view-runs\" class=\"view\">\n");
    out.push_str("<h2>Runs</h2>\n");
    out.push_str(
        "<p class=\"note\">Per run directory, each config's per-task \
         <b>reward / recall / fp</b>. The arena is read from the trials, so a \
         directory whose name disagrees is flagged.</p>\n",
    );

    if runs.is_empty() {
        out.push_str("<p class=\"empty\">No run directories read.</p>\n</section>\n");
        return;
    }

    for run in runs {
        let mismatch = !run.dir.contains(&run.arena_id);
        out.push_str(&format!(
            "<section class=\"run\" id=\"run-{}\">\n",
            slug(&run.dir, "")
        ));
        out.push_str(&format!(
            "<div class=\"run-h\"><code>{}</code><span class=\"arena\">{}</span>\
             <span class=\"ver\">{}</span>{}</div>\n",
            escape_html(&run.dir),
            escape_html(&run.arena_id),
            escape_html(&run.arena_version),
            if mismatch {
                "<span class=\"flag\">name≠arena</span>"
            } else {
                ""
            },
        ));

        for config in &run.configs {
            out.push_str("<div class=\"runcfg\">\n");
            out.push_str(&format!(
                "<div class=\"runcfg-h\"><span class=\"cfg\">{}</span>\
                 <span class=\"kind\">{}</span><code class=\"hash\">{}</code></div>\n",
                escape_html(&config.id),
                escape_html(&config.kind),
                escape_html(&config.composition_hash),
            ));
            out.push_str("<div class=\"grid\">\n");
            for cell in &config.cells {
                let tone = tone_for(cell.reward);
                out.push_str(&format!(
                    "<div class=\"cell {tone}\"><span class=\"ct\">{}</span>\
                     <span class=\"cr\">{}</span>\
                     <span class=\"cd\">rec {} · fp {}{}</span></div>\n",
                    escape_html(&cell.task_id),
                    fmt2(cell.reward),
                    fmt2(cell.recall),
                    fmt_fp(cell.false_positives),
                    if cell.n_errors > 0 {
                        format!(" · {}err", cell.n_errors)
                    } else {
                        String::new()
                    },
                ));
            }
            out.push_str("</div>\n</div>\n");
        }
        out.push_str("</section>\n");
    }
    out.push_str("</section>\n");
}

/// A fixed legend: what each badge and interval means, so the page is
/// self-explanatory on a phone with no other context.
fn render_legend(out: &mut String, board: &Leaderboard) {
    out.push_str("<section class=\"legend\">\n<h2>How to read this</h2>\n<ul>\n");
    out.push_str(
        "<li><b>reward</b> is a continuous score in 0..1 (partial credit is normal); \
         its interval is a <b>percentile bootstrap</b> resampled over whole tasks — \
         the variance that generalizes.</li>\n",
    );
    out.push_str(
        "<li><b>solve rate</b> is the fraction of <b>tasks</b> fully solved (every trial \
         earned a full reward); its interval is a <b>Wilson</b> score interval over tasks — \
         the task is the unit of independence, so correlated trials are not double-counted.</li>\n",
    );
    out.push_str(
        "<li>The rank-gap verdict has three honest states. <b>≫ stronger than runner-up</b> \
         (or <b>≪ runner-up is stronger</b>): the <i>directional</i> gap cleared the noise floor \
         — McNemar on paired solves <i>and</i> a seed-stable paired reward bootstrap must agree, \
         each at α/2 (family-wise α=0.05). <b>≈ inside noise floor</b>: tested, but the gap is not \
         defensible on this data. <b>insufficient data</b>: fewer than 6 shared tasks — too little \
         to test. The bootstrap verdict is a 64-seed envelope, so it does not change with the \
         random seed.</li>\n",
    );
    out.push_str(
        "<li>A reward is only comparable <b>within one arena version</b>. Versions are \
         never pooled and never compared side by side.</li>\n",
    );
    out.push_str(&format!(
        "<li>Every interval is at <b>{:.0}%</b> and every bootstrap is seeded \
         ({} resamples, seed {:#018x}), so the same corpus reproduces this page exactly.</li>\n",
        board.confidence * 100.0,
        board.resamples,
        board.seed,
    ));
    out.push_str("</ul>\n</section>\n");
}

// ----- formatting + escaping helpers -------------------------------------------

/// Clamp a value into the `0..=1` track a bar lives in.
fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

/// Two-decimal fixed format for a reward/CI bound, e.g. `0.83`.
fn fmt2(x: f64) -> String {
    format!("{x:.2}")
}

/// A reward-mean difference with an explicit sign, e.g. `+0.42` / `-0.10`.
fn signed(x: f64) -> String {
    format!("{x:+.2}")
}

/// A proportion as a whole-ish percent, e.g. `62%`.
fn pct(x: f64) -> String {
    format!("{:.0}%", clamp01(x) * 100.0)
}

/// A mean false-positive count: integer when whole, else one decimal.
fn fmt_fp(x: f64) -> String {
    if (x - x.round()).abs() < 1e-9 {
        (x.round() as i64).to_string()
    } else {
        format!("{x:.1}")
    }
}

/// A p-value: `<0.001` below that floor, else three decimals.
fn fmt_p(p: f64) -> String {
    if p < 0.001 {
        "<0.001".to_string()
    } else {
        format!("{p:.3}")
    }
}

/// A reward tone bucket for a drill-down cell: `bad` / `mid` / `good`.
fn tone_for(reward: f64) -> &'static str {
    if reward >= 0.999 {
        "good"
    } else if reward >= 0.5 {
        "mid"
    } else {
        "bad"
    }
}

/// `""` or `"s"` for an English plural on a count.
fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// A DOM-id-safe slug from an arena id + version (or any pair); non-alphanumerics
/// collapse to `-`. An empty second part is omitted, so `slug(dir, "")` slugs a
/// lone string.
fn slug(a: &str, b: &str) -> String {
    let joined = if b.is_empty() {
        a.to_string()
    } else {
        format!("{a}-{b}")
    };
    let mut s = String::with_capacity(joined.len());
    for ch in joined.chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
        } else {
            s.push('-');
        }
    }
    s
}

/// Escape the five HTML-significant characters so any arena/config/task/run string
/// renders as text, never markup.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Inlined stylesheet — phone-first, dark, dense. No external asset, no CDN.
const STYLE: &str = r#"
:root{
  --bg:#0b0e14; --panel:#121724; --panel2:#0f1420; --line:#222a3b;
  --ink:#d8deea; --mut:#8a94a8; --acc:#4f9cf9; --good:#3fb950; --warn:#d6a429;
  --bad:#f0603a;
}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{
  margin:0; background:var(--bg); color:var(--ink);
  font:14px/1.45 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;
  padding-bottom:48px;
}
code{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:.82em}
h1,h2,h3{margin:0;font-weight:650;letter-spacing:-.01em}
.top{padding:18px 16px 12px;border-bottom:1px solid var(--line);
  background:linear-gradient(180deg,#121828,#0b0e14)}
.top h1{font-size:22px;display:flex;align-items:baseline;gap:8px}
.ae-icon{width:.78em;height:.78em;align-self:center;stroke:currentColor;stroke-width:1.6;stroke-linecap:round;stroke-linejoin:round;fill:none;flex:none}
.top h1 .sub{font-size:12px;color:var(--mut);font-weight:500;letter-spacing:.02em;text-transform:uppercase}
.prov{display:flex;flex-wrap:wrap;gap:8px 14px;margin-top:10px;color:var(--mut);font-size:12.5px}
.prov b{color:var(--ink);font-weight:650}
.prov .warn b{color:var(--warn)}
.prov .ci{color:var(--mut)}
.paths{margin-top:7px;color:var(--mut);font-size:11px;word-break:break-all}
.paths code{color:#9fb2cf}
nav.tabs{position:sticky;top:0;z-index:5;display:flex;gap:2px;padding:6px;
  background:rgba(11,14,20,.92);backdrop-filter:blur(8px);border-bottom:1px solid var(--line)}
nav.tabs a{flex:1;text-align:center;padding:10px 8px;border-radius:9px;color:var(--mut);
  text-decoration:none;font-weight:600;font-size:13px}
nav.tabs a.active{background:var(--panel);color:var(--ink);box-shadow:inset 0 0 0 1px var(--line)}
main{padding:14px 12px 0}
body.tabbed .view{display:none}
body.tabbed .view.active{display:block}
.view>h2,.legend h2{font-size:15px;margin:4px 2px 10px;color:var(--ink)}
.note{color:var(--mut);font-size:12px;margin:0 2px 12px}
.note code{color:#9fb2cf}
.empty,.muted{color:var(--mut)}
.empty{padding:18px 2px}
.cards{display:grid;grid-template-columns:1fr;gap:10px}
@media(min-width:560px){.cards{grid-template-columns:1fr 1fr}}
.card{display:block;padding:12px 13px;background:var(--panel);border:1px solid var(--line);
  border-radius:13px;text-decoration:none;color:inherit}
.card:active{background:var(--panel2)}
.card-h{display:flex;align-items:baseline;gap:8px;flex-wrap:wrap}
.arena{font-weight:650;font-size:14.5px}
.ver{font-size:11px;color:#bcd0ee;background:#16263f;border:1px solid #1e3553;
  padding:1px 7px;border-radius:999px;font-variant-numeric:tabular-nums}
.ver.big{font-size:12.5px;padding:2px 9px}
.card-m{display:flex;gap:12px;color:var(--mut);font-size:12px;margin-top:7px}
.card-top{display:flex;justify-content:space-between;align-items:center;margin-top:9px;
  padding-top:9px;border-top:1px solid var(--line)}
.card-top .lead{font-size:12.5px;color:#cdd6e6;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.card-top .rew{font-weight:700;font-variant-numeric:tabular-nums;color:var(--good)}
.card-top.muted{color:var(--mut);font-size:12px}
.eval{margin:0 0 22px;padding:13px;background:var(--panel);border:1px solid var(--line);border-radius:14px}
.eval-h{display:flex;align-items:baseline;gap:9px;flex-wrap:wrap;padding-bottom:6px;margin-bottom:6px;
  border-bottom:1px solid var(--line)}
.eval-h .arena{font-size:16px}
.block{margin-top:13px}
.block h3{font-size:12px;text-transform:uppercase;letter-spacing:.04em;color:var(--mut);margin-bottom:7px}
.chips{display:flex;flex-wrap:wrap;gap:6px}
.chip{display:inline-flex;flex-direction:column;line-height:1.25;background:var(--panel2);
  border:1px solid var(--line);border-radius:8px;padding:4px 8px;font-size:12.5px}
.chip small{color:var(--mut);font-size:10.5px}
.runlist{display:flex;flex-direction:column;gap:5px}
.runref{display:flex;align-items:center;gap:9px;flex-wrap:wrap;font-size:11.5px;color:var(--mut)}
.runref code{color:#9fb2cf;word-break:break-all}
.flag{color:#0b0e14;background:var(--warn);border-radius:5px;padding:0 6px;font-size:10px;font-weight:700}
.board{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:9px}
.row{background:var(--panel2);border:1px solid var(--line);border-radius:12px;padding:10px 11px}
.row-h{display:flex;align-items:baseline;gap:8px;flex-wrap:wrap;margin-bottom:8px}
.rank{font-weight:800;color:var(--acc);font-variant-numeric:tabular-nums}
.cfg{font-weight:650;word-break:break-word}
.kind{font-size:11px;color:var(--mut);background:#10182a;border:1px solid var(--line);
  padding:0 6px;border-radius:999px}
.hash{color:#7f8aa3;margin-left:auto}
.metric{display:grid;grid-template-columns:64px 1fr 92px;align-items:center;gap:9px;margin:5px 0}
.metric-l{font-size:11px;color:var(--mut);line-height:1.15}
.metric-l small{display:block;font-size:9.5px;opacity:.8}
.metric-v{text-align:right;font-weight:700;font-variant-numeric:tabular-nums;font-size:13px}
.metric-v small{display:block;font-size:10px;color:var(--mut);font-weight:500}
.bar{position:relative;height:18px;background:#0a0f1a;border:1px solid var(--line);border-radius:6px;overflow:hidden}
.bar .fill{position:absolute;top:0;bottom:0;left:0;border-radius:5px 0 0 5px}
.bar.bootstrap .fill{background:linear-gradient(90deg,#1d4ed8,#3fb950)}
.bar.wilson .fill{background:linear-gradient(90deg,#274b78,#4f9cf9)}
.bar .ciband{position:absolute;top:3px;bottom:3px;background:rgba(216,222,234,.16);
  border-left:1px solid rgba(216,222,234,.5);border-right:1px solid rgba(216,222,234,.5)}
.bar .whisk{position:absolute;top:2px;bottom:2px;width:0;border-left:1.5px solid #e7ecf5}
.bar .pt{position:absolute;top:-1px;bottom:-1px;width:0;border-left:2px solid #fff}
.n{color:var(--mut);font-size:11px;margin:7px 0 8px}
.verdict{font-size:11.5px;border-radius:8px;padding:6px 9px;line-height:1.3}
.verdict small{display:block;font-size:10px;opacity:.92;margin-top:2px;font-variant-numeric:tabular-nums}
.verdict.signal{background:rgba(63,185,80,.12);border:1px solid rgba(63,185,80,.4);color:#9fe6ab}
.verdict.noise{background:rgba(214,164,41,.1);border:1px solid rgba(214,164,41,.36);color:#e6cf93}
.verdict.base{background:#10182a;border:1px solid var(--line);color:var(--mut)}
.verdict.thin{background:rgba(110,118,138,.1);border:1px dashed var(--line);color:var(--mut)}
.run{margin:0 0 16px;padding:12px;background:var(--panel);border:1px solid var(--line);border-radius:13px}
.run-h{display:flex;align-items:center;gap:8px;flex-wrap:wrap;padding-bottom:8px;margin-bottom:8px;
  border-bottom:1px solid var(--line)}
.run-h code{color:#9fb2cf;word-break:break-all;font-size:11.5px}
.runcfg{margin-top:10px}
.runcfg-h{display:flex;align-items:baseline;gap:8px;flex-wrap:wrap;margin-bottom:6px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(96px,1fr));gap:6px}
.cell{background:var(--panel2);border:1px solid var(--line);border-radius:8px;padding:6px 7px;
  display:flex;flex-direction:column;gap:1px;border-left-width:3px}
.cell.good{border-left-color:var(--good)}
.cell.mid{border-left-color:var(--warn)}
.cell.bad{border-left-color:var(--bad)}
.cell .ct{font-size:10.5px;color:var(--mut);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.cell .cr{font-weight:700;font-variant-numeric:tabular-nums;font-size:14px}
.cell .cd{font-size:9.5px;color:var(--mut);font-variant-numeric:tabular-nums}
.legend{margin:22px 0 0;padding:14px 13px;background:var(--panel);border:1px solid var(--line);
  border-radius:14px}
.legend ul{margin:6px 0 0;padding-left:18px;color:var(--mut);font-size:12px}
.legend li{margin:6px 0}
.legend b{color:var(--ink)}
"#;

/// Inlined progressive-enhancement script: turn the three sections into tabs and
/// wire the overview cards to jump into the detail view. With scripting off the
/// body never gets `tabbed`, so every section stays visible in one scroll.
const SCRIPT: &str = r#"
(function(){
  var body=document.body; body.classList.add('tabbed');
  var views=[].slice.call(document.querySelectorAll('.view'));
  var tabs=[].slice.call(document.querySelectorAll('nav.tabs a'));
  function show(id){
    views.forEach(function(v){ v.classList.toggle('active', v.id===id); });
    tabs.forEach(function(t){ t.classList.toggle('active', t.getAttribute('data-view')===id); });
    window.scrollTo(0,0);
  }
  tabs.forEach(function(t){
    t.addEventListener('click', function(e){ e.preventDefault(); show(t.getAttribute('data-view')); });
  });
  [].slice.call(document.querySelectorAll('[data-jump]')).forEach(function(a){
    a.addEventListener('click', function(e){
      e.preventDefault();
      show('view-detail');
      var el=document.getElementById(a.getAttribute('data-jump'));
      if(el){ el.scrollIntoView({behavior:'smooth',block:'start'});
        el.classList.add('hit'); setTimeout(function(){ el.classList.remove('hit'); },1200); }
    });
  });
  show('view-evals');
})();
"#;
