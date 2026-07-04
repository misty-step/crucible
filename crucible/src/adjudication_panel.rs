//! Phone-first renderer for a [`JudgmentQueue`], in two modes.
//!
//! [`render`]/[`write_panel`] are the original static projection: no store, no
//! server, no hidden write path. [`render_live`] is the same markup wired for
//! [`crate::adjudication_server`]'s minimal local writeback loop (backlog 005)
//! — each verdict button posts to `/label` instead of doing nothing. Both
//! preserve the shipped `crucible.judgment_queue.v1` / `crucible.label.v1`
//! contract as the narrow waist; `render_live` adds no new data model, only a
//! `fetch()` call.

use std::path::Path;

use anyhow::Context;
use crucible_core::{JudgmentItem, JudgmentQueue, Label};

use crate::{first_line_truncated, load_queue, location_label};

/// Render a judgment queue artifact into `<out>/index.html` plus a copied
/// `<out>/queue.json` model for inspection.
pub fn write_panel(queue_path: &Path, out: &Path) -> anyhow::Result<PanelReceipt> {
    let queue = load_queue(queue_path)?;
    std::fs::create_dir_all(out)
        .with_context(|| format!("creating panel output directory {}", out.display()))?;

    let queue_json = serde_json::to_string_pretty(&queue).context("serializing queue model")?;
    let queue_out = out.join("queue.json");
    std::fs::write(&queue_out, format!("{queue_json}\n"))
        .with_context(|| format!("writing {}", queue_out.display()))?;

    let html_out = out.join("index.html");
    std::fs::write(&html_out, render(&queue))
        .with_context(|| format!("writing {}", html_out.display()))?;

    Ok(PanelReceipt {
        items: queue.items.len(),
        labels: queue.labels.len(),
        html_path: html_out,
        queue_path: queue_out,
    })
}

/// Paths and counts emitted by [`write_panel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelReceipt {
    pub items: usize,
    pub labels: usize,
    pub html_path: std::path::PathBuf,
    pub queue_path: std::path::PathBuf,
}

/// Render the static (non-interactive) panel HTML: verdict buttons present,
/// wired to nothing. What [`write_panel`] has always produced.
pub fn render(queue: &JudgmentQueue) -> String {
    render_shell(queue, false)
}

/// Render the panel HTML wired for [`crate::adjudication_server`]'s live
/// writeback loop: each verdict button posts a decision to `POST /label` and
/// the page reflects the response without a reload. Identical markup and data
/// model to [`render`] otherwise — the only new surface is the `fetch()` call.
pub fn render_live(queue: &JudgmentQueue) -> String {
    render_shell(queue, true)
}

fn render_shell(queue: &JudgmentQueue, live: bool) -> String {
    let labeled = queue.labels.len();
    let total = queue.items.len();
    let remaining = total.saturating_sub(labeled);
    let progress = if total == 0 {
        0.0
    } else {
        labeled as f64 / total as f64
    };

    let mut html = String::new();
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("<meta charset=\"utf-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    html.push_str("<title>Crucible Adjudication Queue</title>\n");
    html.push_str(r#"<link rel="icon" type="image/svg+xml" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%231a1a1a' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2'/%3E%3Cpath d='M6.453 15h11.094'/%3E%3Cpath d='M8.5 2h7'/%3E%3C/svg%3E">"#);
    html.push_str("\n");
    html.push_str("<style>\n");
    html.push_str(
        ":root{--bg:#f6f1e8;--panel:#fffaf0;--ink:#231f18;--muted:#6a6258;--line:#dfd3bf;--red:#a73a2a;--teal:#247467;--gold:#a77a24;--ok:#1f7a4f}\
         @media (prefers-color-scheme:dark){:root{--bg:#101319;--panel:#171b22;--ink:#eee5d6;--muted:#a39b90;--line:#2b313b;--red:#e3674f;--teal:#57baa5;--gold:#d5ad51;--ok:#5fc68e}}\
         *{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:16px/1.5 -apple-system,BlinkMacSystemFont,\"Segoe UI\",Roboto,Helvetica,Arial,sans-serif;padding:0 0 4rem}\
         header{position:sticky;top:0;background:color-mix(in srgb,var(--bg) 92%,transparent);backdrop-filter:blur(8px);border-bottom:1px solid var(--line);z-index:2}\
         .wrap{max-width:44rem;margin:0 auto;padding:1rem}.eyebrow{font:700 .68rem/1 ui-monospace,SFMono-Regular,Menlo,monospace;letter-spacing:.12em;text-transform:uppercase;color:var(--red)}\
         h1{font-size:1.35rem;line-height:1.15;margin:.25rem 0 .65rem}.status{display:grid;grid-template-columns:repeat(3,1fr);gap:.5rem}.stat{border:1px solid var(--line);background:var(--panel);border-radius:.45rem;padding:.5rem}.stat b{display:block;font-size:1.15rem}.stat span{font-size:.72rem;color:var(--muted)}\
         .bar{height:.5rem;border-radius:1rem;background:color-mix(in srgb,var(--muted) 18%,transparent);overflow:hidden;margin:.8rem 0 .1rem}.bar i{display:block;height:100%;background:var(--teal)}\
         main{max-width:44rem;margin:0 auto;padding:.75rem 1rem}.item{background:var(--panel);border:1px solid var(--line);border-radius:.55rem;margin:.85rem 0;padding:.9rem}.item.recoverable{border-left:4px solid var(--gold)}.item.plain{border-left:4px solid var(--teal)}\
         .meta{display:flex;gap:.45rem;flex-wrap:wrap;margin-bottom:.55rem}.pill{font:700 .68rem/1 ui-monospace,SFMono-Regular,Menlo,monospace;border:1px solid var(--line);border-radius:999px;padding:.28rem .48rem;color:var(--muted)}\
         .loc{color:var(--teal)}.claim{margin:.55rem 0 .7rem}.against{border-top:1px dashed var(--line);padding-top:.55rem;color:var(--muted);font-size:.9rem}.actions{display:grid;grid-template-columns:repeat(4,1fr);gap:.45rem;margin-top:.75rem}\
         button{appearance:none;border:1px solid var(--line);border-radius:.45rem;background:var(--bg);color:var(--ink);font-weight:700;padding:.65rem .35rem;min-height:2.7rem}button.keep{border-color:var(--ok)}button.nit{border-color:var(--gold)}button.wrong,button.noise{border-color:var(--red)}\
         .labels{margin-top:.7rem;border-top:1px dashed var(--line);padding-top:.55rem;color:var(--muted);font-size:.9rem}.empty{border:1px solid var(--line);background:var(--panel);border-radius:.55rem;padding:1rem;color:var(--muted)}",
    );
    html.push_str("</style>\n</head>\n<body>\n<header><div class=\"wrap\">\n");
    html.push_str("<div class=\"eyebrow\">Crucible judgment queue</div>\n");
    html.push_str("<h1>Adjudication panel</h1>\n");
    html.push_str("<div class=\"status\">");
    html.push_str(&stat("Items", total));
    html.push_str(&stat("Labeled", labeled));
    html.push_str(&stat("Open", remaining));
    html.push_str("</div>");
    html.push_str(&format!(
        "<div class=\"bar\" aria-label=\"Progress\"><i style=\"width:{:.2}%\"></i></div>",
        progress * 100.0
    ));
    html.push_str("</div></header>\n<main>\n");

    if queue.items.is_empty() {
        html.push_str("<section class=\"empty\">No disputed findings.</section>\n");
    } else {
        for item in &queue.items {
            html.push_str(&render_item(item, &queue.labels));
        }
    }

    html.push_str("</main>\n");
    if live {
        html.push_str(LIVE_SCRIPT);
    }
    html.push_str("</body>\n</html>\n");
    html
}

/// Wires every `.item .actions button[data-verdict]` to `POST /label`
/// ([`crate::adjudication_server`]) on click: computes `latency_ms` from page
/// load to click, disables the item's buttons during the request, and on
/// success replaces the item's actions with the returned label so a re-click
/// isn't possible without a reload. `saw_grader_before_commit` is always
/// `true` here — the panel shows the deterministic grader's context (category,
/// recoverable-against rows) inline before every verdict, so that is the
/// honest value for this surface, not a default to override later.
const LIVE_SCRIPT: &str = r#"<script>
(function () {
  var renderedAt = Date.now();
  document.querySelectorAll('.item .actions button[data-verdict]').forEach(function (button) {
    button.addEventListener('click', function () {
      var item = button.closest('.item');
      var findingId = item.getAttribute('data-finding-id');
      var actions = item.querySelector('.actions');
      actions.querySelectorAll('button').forEach(function (b) { b.disabled = true; });
      fetch('/label', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          finding_id: findingId,
          verdict: button.getAttribute('data-verdict'),
          in_scope: true,
          latency_ms: Date.now() - renderedAt
        })
      }).then(function (res) {
        if (!res.ok) { return res.text().then(function (text) { throw new Error(text); }); }
        return res.json();
      }).then(function (data) {
        var note = document.createElement('div');
        note.className = 'labels';
        note.textContent = 'Label: ' + data.label.verdict + ' · saved (' + data.labeled + '/' + data.total + ')';
        actions.replaceWith(note);
      }).catch(function (err) {
        actions.querySelectorAll('button').forEach(function (b) { b.disabled = false; });
        var note = document.createElement('div');
        note.className = 'labels';
        note.textContent = 'Save failed: ' + err.message;
        actions.after(note);
      });
    });
  });
})();
</script>
"#;

fn stat(label: &str, n: usize) -> String {
    format!(
        "<div class=\"stat\"><b>{}</b><span>{}</span></div>",
        n,
        escape_html(label)
    )
}

fn render_item(item: &JudgmentItem, labels: &[Label]) -> String {
    let kind = if item.is_recoverable() {
        "recoverable"
    } else {
        "plain"
    };
    let mut html = String::new();
    html.push_str(&format!(
        "<section class=\"item {kind}\" data-finding-id=\"{}\">\n",
        escape_html(&item.finding_id)
    ));
    html.push_str("<div class=\"meta\">");
    html.push_str(&pill(&item.finding_id));
    html.push_str(&pill(if item.is_recoverable() {
        "recoverable"
    } else {
        "dispute"
    }));
    html.push_str(&format!(
        "<span class=\"pill loc\">{}</span>",
        escape_html(&location_label(&item.candidate))
    ));
    html.push_str(&pill(&item.candidate.category));
    html.push_str("</div>\n");
    html.push_str(&format!(
        "<p class=\"claim\">{}</p>\n",
        escape_html(&first_line_truncated(&item.candidate.description, 180))
    ));
    if !item.recoverable_against.is_empty() {
        html.push_str("<div class=\"against\">");
        for key in &item.recoverable_against {
            html.push_str(&format!(
                "<div>Near key row: {} · {}</div>",
                escape_html(&location_label(key)),
                escape_html(&key.category)
            ));
        }
        html.push_str("</div>\n");
    }
    html.push_str("<div class=\"actions\" aria-label=\"Verdicts\">");
    html.push_str("<button class=\"keep\" type=\"button\" data-verdict=\"keep\">Keep</button>");
    html.push_str("<button class=\"nit\" type=\"button\" data-verdict=\"nit\">Nit</button>");
    html.push_str("<button class=\"wrong\" type=\"button\" data-verdict=\"wrong\">Wrong</button>");
    html.push_str("<button class=\"noise\" type=\"button\" data-verdict=\"noise\">Noise</button>");
    html.push_str("</div>\n");

    let item_labels: Vec<&Label> = labels
        .iter()
        .filter(|label| label.finding_id == item.finding_id)
        .collect();
    if !item_labels.is_empty() {
        html.push_str("<div class=\"labels\">");
        for label in item_labels {
            html.push_str(&format!(
                "<div>Label: {:?} · in_scope={} · blind={}</div>",
                label.verdict, label.disposition.in_scope, !label.saw_grader_before_commit
            ));
        }
        html.push_str("</div>\n");
    }

    html.push_str("</section>\n");
    html
}

fn pill(text: &str) -> String {
    format!("<span class=\"pill\">{}</span>", escape_html(text))
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
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

#[cfg(test)]
mod tests {
    use crucible_core::{Disposition, GradeSummary, KeyFinding, Label, Verdict, LABEL_SCHEMA};

    use super::*;

    fn item(id: &str) -> JudgmentItem {
        JudgmentItem {
            finding_id: id.to_string(),
            candidate: KeyFinding {
                file: "cache.py".to_string(),
                line: 23,
                category: "concurrency".to_string(),
                severity: "blocking".to_string(),
                description: "Concurrent writers share one temp file.".to_string(),
                source_id: Some(id.to_string()),
            },
            recoverable_against: vec![KeyFinding {
                file: "cache.py".to_string(),
                line: 23,
                category: "resource-leak".to_string(),
                severity: String::new(),
                description: "co-located key".to_string(),
                source_id: None,
            }],
        }
    }

    #[test]
    fn panel_renders_queue_items_and_verdict_controls() {
        let queue = JudgmentQueue {
            schema_version: crucible_core::JUDGMENT_QUEUE_SCHEMA.to_string(),
            summary: GradeSummary {
                matched: 1,
                disputed: 1,
                missed: 1,
                recoverable_misses: 1,
            },
            items: vec![item("F3")],
            labels: vec![Label {
                schema_version: LABEL_SCHEMA.to_string(),
                finding_id: "F3".to_string(),
                verdict: Verdict::Keep,
                disposition: Disposition { in_scope: true },
                latency_ms: 90_000,
                saw_grader_before_commit: false,
                timestamp: String::new(),
            }],
        };

        let html = render(&queue);
        for marker in [
            "name=\"viewport\"",
            "F3",
            "cache.py:23",
            "recoverable",
            "Keep",
            "Nit",
            "Wrong",
            "Noise",
            "Label: Keep",
        ] {
            assert!(html.contains(marker), "missing {marker:?} in {html}");
        }
    }
}
