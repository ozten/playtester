//! "What changed" markdown reporter for Phase 6 compare.
//!
//! Turns a [`CompareResult`] into the section layout sketched in
//! the plan:
//!
//! - `## Compare:` header with sample sizes, correction, alpha, and
//!   finding-count summary.
//! - Flagged findings split into two tables: "variant is larger"
//!   (delta > 0) and "variant is smaller" (delta < 0). Sorted by
//!   |delta| desc.
//! - Per-agent win-rate deltas as their own subsection.
//! - Subjective deltas (Phase 5 Likert + coded tags), gated on
//!   critique availability.
//! - "Rejected (correction noise)" folded summary.
//! - Only-in-one-side metrics as a compact bullet list.
//!
//! Empty sections are omitted. Sample-size imbalance (ratio ≥ 2×)
//! surfaces as a banner at the top so comparisons over mismatched
//! cohorts don't silently mislead.

use super::engine::{CompareResult, CritiqueAvailability, Finding, FindingKind};
use crate::markdown::MarkdownBuilder;

const IMBALANCE_RATIO: f64 = 2.0;
const REJECTED_TOP_K: usize = 5;
const UNCHANGED_TOP_K: usize = 5;

/// Render a [`CompareResult`] into `md`. Always writes a top-level
/// heading so the report has a clear anchor even for empty results.
#[allow(clippy::too_many_lines)]
pub fn write_compare_report(md: &mut MarkdownBuilder, result: &CompareResult) {
    md.h2("Compare");

    // --- Header: sample sizes, correction, totals ---
    md.bullet(&format!(
        "Games: baseline = **{}**, variant = **{}**",
        result.n_baseline, result.n_variant
    ));
    md.bullet(&format!(
        "Correction: **{}** @ α = **{}**",
        result.opts.correction, result.opts.alpha
    ));
    md.bullet(&format!(
        "Findings: **{}** flagged, **{}** rejected (correction noise), **{}** unchanged",
        result.flagged.len(),
        result.rejected.len(),
        result.unchanged.len()
    ));
    md.end_block();

    // --- Sample-size imbalance banner ---
    if let Some(ratio) = size_ratio(result.n_baseline, result.n_variant)
        && ratio >= IMBALANCE_RATIO
    {
        md.paragraph(&format!(
            "**Warning:** sample sizes differ by {ratio:.1}× (baseline = {}, variant = {}). \
             Compare results over imbalanced cohorts with care — small-sample cohorts have \
             wider confidence intervals.",
            result.n_baseline, result.n_variant
        ));
    }

    // --- No findings at all ---
    if result.flagged.is_empty()
        && result.rejected.is_empty()
        && result.unchanged.is_empty()
        && result.only_baseline.is_empty()
        && result.only_variant.is_empty()
    {
        md.paragraph("*No comparable data.*");
        return;
    }

    // --- Flagged — split by direction ---
    let (larger, smaller): (Vec<&Finding>, Vec<&Finding>) = result
        .flagged
        .iter()
        .filter(|f| f.kind == FindingKind::NumericMetric || f.kind == FindingKind::LikertQuestion)
        .partition(|f| f.outcome.delta > 0.0);

    if !larger.is_empty() {
        md.h3("Flagged: variant > baseline");
        render_findings_table(md, &larger);
    }
    if !smaller.is_empty() {
        md.h3("Flagged: variant < baseline");
        render_findings_table(md, &smaller);
    }

    // --- Per-agent win-rate deltas ---
    let agent_flagged: Vec<&Finding> = result
        .flagged
        .iter()
        .filter(|f| f.kind == FindingKind::AgentWinRate)
        .collect();
    let agent_other: Vec<&Finding> = result
        .rejected
        .iter()
        .chain(result.unchanged.iter())
        .filter(|f| f.kind == FindingKind::AgentWinRate)
        .collect();
    if !agent_flagged.is_empty() || !agent_other.is_empty() {
        md.h3("Per-agent win-rate deltas");
        render_agent_table(md, &agent_flagged, &agent_other);
    }

    // --- Subjective deltas (gated on critique availability) ---
    match result.critique {
        CritiqueAvailability::Neither => {}
        CritiqueAvailability::OnlyBaseline => {
            md.h3("Subjective deltas");
            md.paragraph(
                "*Baseline has critique data but variant does not — subjective diffs skipped.*",
            );
        }
        CritiqueAvailability::OnlyVariant => {
            md.h3("Subjective deltas");
            md.paragraph(
                "*Variant has critique data but baseline does not — subjective diffs skipped.*",
            );
        }
        CritiqueAvailability::Both => {
            let likert: Vec<&Finding> = result
                .flagged
                .iter()
                .chain(result.rejected.iter())
                .chain(result.unchanged.iter())
                .filter(|f| f.kind == FindingKind::LikertQuestion)
                .collect();
            let tags: Vec<&Finding> = result
                .flagged
                .iter()
                .chain(result.rejected.iter())
                .chain(result.unchanged.iter())
                .filter(|f| f.kind == FindingKind::CodedTag)
                .collect();
            if !likert.is_empty() || !tags.is_empty() {
                md.h3("Subjective deltas");
                if !likert.is_empty() {
                    md.paragraph("**Likert (Welch's t-test on per-seat scores):**");
                    render_findings_table(md, &likert);
                }
                if !tags.is_empty() {
                    md.paragraph(
                        "**Coded tags (two-proportion z-test on per-critique mention):**",
                    );
                    render_findings_table(md, &tags);
                }
            }
        }
    }

    // --- Rejected (correction noise) ---
    if !result.rejected.is_empty() {
        md.h3("Rejected (correction noise)");
        md.paragraph(&format!(
            "Significant at raw α = {} but rejected by {} correction.",
            result.opts.alpha, result.opts.correction
        ));
        let top: Vec<&Finding> = result.rejected.iter().take(REJECTED_TOP_K).collect();
        render_findings_table(md, &top);
        if result.rejected.len() > REJECTED_TOP_K {
            md.bullet(&format!(
                "… plus {} more rejected findings.",
                result.rejected.len() - REJECTED_TOP_K
            ));
            md.end_block();
        }
    }

    // --- Unchanged (folded summary) ---
    if !result.unchanged.is_empty() {
        md.h3("Unchanged (near-miss folded summary)");
        md.paragraph(&format!(
            "{} metrics fell below both raw α and correction threshold. Top-{} nearest-to-significant listed below.",
            result.unchanged.len(),
            UNCHANGED_TOP_K.min(result.unchanged.len())
        ));
        let top: Vec<&Finding> = result.unchanged.iter().take(UNCHANGED_TOP_K).collect();
        render_findings_table(md, &top);
    }

    // --- Only in one side ---
    if !result.only_baseline.is_empty() || !result.only_variant.is_empty() {
        md.h3("Metrics present in only one cohort");
        if !result.only_baseline.is_empty() {
            md.bullet(&format!(
                "**Only in baseline** ({}): {}",
                result.only_baseline.len(),
                result
                    .only_baseline
                    .iter()
                    .map(super::MetricKey::label)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !result.only_variant.is_empty() {
            md.bullet(&format!(
                "**Only in variant** ({}): {}",
                result.only_variant.len(),
                result
                    .only_variant
                    .iter()
                    .map(super::MetricKey::label)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        md.end_block();
    }
}

fn render_findings_table(md: &mut MarkdownBuilder, findings: &[&Finding]) {
    let headers = &["metric", "baseline", "variant", "delta", "p (raw)", "n_b / n_v"];
    let rows: Vec<Vec<String>> = findings
        .iter()
        .map(|f| {
            vec![
                f.label.clone(),
                format!("{:.4}", f.outcome.mean_b),
                format!("{:.4}", f.outcome.mean_a),
                format!("{:+.4}", f.outcome.delta),
                format_p(f.outcome.p_value),
                format!("{} / {}", f.outcome.n_b, f.outcome.n_a),
            ]
        })
        .collect();
    md.table(headers, &rows);
}

fn render_agent_table(
    md: &mut MarkdownBuilder,
    flagged: &[&Finding],
    other: &[&Finding],
) {
    let headers = &[
        "agent",
        "baseline wr",
        "variant wr",
        "delta",
        "p (raw)",
        "flagged",
    ];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for f in flagged.iter().chain(other.iter()) {
        rows.push(vec![
            f.label.clone(),
            format!("{:.1}%", f.outcome.mean_b * 100.0),
            format!("{:.1}%", f.outcome.mean_a * 100.0),
            format!("{:+.1}pp", f.outcome.delta * 100.0),
            format_p(f.outcome.p_value),
            if f.significant { "★".into() } else { " ".into() },
        ]);
    }
    md.table(headers, &rows);
}

fn format_p(p: f64) -> String {
    if p.is_nan() {
        "—".to_owned()
    } else if p < 0.001 {
        "< 0.001".to_owned()
    } else {
        format!("{p:.3}")
    }
}

fn size_ratio(a: u64, b: u64) -> Option<f64> {
    if a == 0 || b == 0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let (af, bf) = (a as f64, b as f64);
    Some(if af > bf { af / bf } else { bf / af })
}
