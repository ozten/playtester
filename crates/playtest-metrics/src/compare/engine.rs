//! Compare engine: ties [`super::MetricKey`] enumeration + sample
//! fetches to [`crate::stats`] tests, then applies a single
//! multiple-comparison correction across every family's p-values.
//!
//! Callers supply two `Connection` handles (one per DB — typically
//! freshly ingested via `ingest_directory` into
//! `Connection::open_in_memory`). Game-name dispatch is the CLI's
//! problem; this module only sees the post-ingest SQLite schema.
//!
//! Convention: **delta = variant − baseline**. Positive means the
//! variant cohort had a larger mean (or proportion). Negative means
//! the variant shrank relative to baseline. The reporter labels these
//! as "variant is larger" vs "variant is smaller" rather than
//! editorializing regression vs improvement — the direction depends
//! on whether larger-is-better, which the reporter doesn't know.

use rusqlite::{Connection, Error as SqliteError};
use serde::{Deserialize, Serialize};

use super::{
    MetricKey, enumerate_paired_metrics, fetch_agent_outcomes, fetch_games_count,
    fetch_likert_questions, fetch_likert_samples, fetch_numeric_samples,
    fetch_tag_totals, fetch_total_critique_responses,
};
use crate::stats::{TestOutcome, benjamini_hochberg, bonferroni, two_proportion_z_test, welch_t_test};

/// Multiple-comparison correction method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Correction {
    /// Benjamini–Hochberg FDR. Default — more power at 50+ metrics.
    BenjaminiHochberg,
    /// Bonferroni. Conservative; use for audits.
    Bonferroni,
}

impl core::fmt::Display for Correction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BenjaminiHochberg => write!(f, "BH"),
            Self::Bonferroni => write!(f, "Bonferroni"),
        }
    }
}

/// Options for [`run_compare`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompareOpts {
    pub alpha: f64,
    pub correction: Correction,
}

impl Default for CompareOpts {
    fn default() -> Self {
        Self {
            alpha: 0.05,
            correction: Correction::BenjaminiHochberg,
        }
    }
}

/// What family does a finding belong to? Determines reporter
/// sectioning and test choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindingKind {
    NumericMetric,
    AgentWinRate,
    LikertQuestion,
    CodedTag,
}

/// One compare finding. Always represents a single test with a
/// before / after / delta / p triple plus its corrected significance
/// boolean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    /// Human-readable label rendered in the report.
    pub label: String,
    /// Underlying test outcome: mean_a / mean_b / delta / p etc.
    /// Note: cohort A is **variant**, cohort B is **baseline**, so
    /// `delta = variant − baseline`.
    pub outcome: TestOutcome,
    /// True if this finding passes the correction threshold at
    /// `CompareOpts::alpha`.
    pub significant: bool,
}

/// Which side has Phase 5 critique data. Controls whether Likert /
/// coded-tag sections appear in the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CritiqueAvailability {
    Both,
    OnlyBaseline,
    OnlyVariant,
    #[default]
    Neither,
}

/// Full compare output. Consumed by [`crate::compare::report`] to
/// render the "what changed" markdown.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompareResult {
    /// Findings that cleared the correction threshold.
    pub flagged: Vec<Finding>,
    /// Findings significant at raw α but rejected by correction.
    pub rejected: Vec<Finding>,
    /// Findings that weren't significant at raw α either.
    pub unchanged: Vec<Finding>,
    /// Numeric metrics present in baseline but not variant.
    pub only_baseline: Vec<MetricKey>,
    /// Numeric metrics present in variant but not baseline.
    pub only_variant: Vec<MetricKey>,
    /// Per-cohort game counts — populates the report header.
    pub n_baseline: u64,
    pub n_variant: u64,
    /// Which side(s) had critique data.
    pub critique: CritiqueAvailability,
    /// Snapshot of the options used to produce this result.
    pub opts: CompareOpts,
}

/// Run the full compare pipeline against two ingested DBs. See
/// module docs for the delta convention.
///
/// # Errors
/// Propagates `rusqlite::Error` from the underlying queries. Stats
/// errors on individual metrics are caught and the metric is
/// recorded as `unchanged` with a NaN p-value (rare edge case for
/// degenerate single-sample cohorts).
#[allow(clippy::too_many_lines)]
pub fn run_compare(
    baseline: &Connection,
    variant: &Connection,
    opts: &CompareOpts,
) -> Result<CompareResult, SqliteError> {
    let n_baseline = fetch_games_count(baseline)?;
    let n_variant = fetch_games_count(variant)?;

    // Temporary accumulator: one per test family, all merged before
    // correction (standard cross-family FDR control).
    let mut candidates: Vec<Finding> = Vec::new();

    // --- Numeric metrics ---
    let paired = enumerate_paired_metrics(baseline, variant)?;
    for key in &paired.paired {
        let a_samples = fetch_numeric_samples(variant, key)?;
        let b_samples = fetch_numeric_samples(baseline, key)?;
        if a_samples.is_empty() || b_samples.is_empty() {
            continue;
        }
        match welch_t_test(&a_samples, &b_samples) {
            Ok(outcome) => candidates.push(Finding {
                kind: FindingKind::NumericMetric,
                label: key.label(),
                outcome,
                significant: false, // filled in post-correction
            }),
            Err(_) => {
                // Degenerate case — surface as unchanged with NaN p.
                candidates.push(unchanged_placeholder(FindingKind::NumericMetric, key.label()));
            }
        }
    }

    // --- Per-agent win rates ---
    let agents_a = fetch_agent_outcomes(variant)?;
    let agents_b = fetch_agent_outcomes(baseline)?;
    let agent_names: std::collections::BTreeSet<String> = agents_a
        .iter()
        .chain(agents_b.iter())
        .map(|(name, _, _)| name.clone())
        .collect();
    for name in &agent_names {
        let a = agents_a
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, w, g)| (*w, *g));
        let b = agents_b
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, w, g)| (*w, *g));
        if let (Some((wa, na)), Some((wb, nb))) = (a, b)
            && na > 0
            && nb > 0
        {
            match two_proportion_z_test(wa, na, wb, nb) {
                Ok(outcome) => candidates.push(Finding {
                    kind: FindingKind::AgentWinRate,
                    label: name.clone(),
                    outcome,
                    significant: false,
                }),
                Err(_) => candidates.push(unchanged_placeholder(
                    FindingKind::AgentWinRate,
                    name.clone(),
                )),
            }
        }
    }

    // --- Critique Likert + tags ---
    let n_crit_a = fetch_total_critique_responses(variant)?;
    let n_crit_b = fetch_total_critique_responses(baseline)?;
    let critique = match (n_crit_a, n_crit_b) {
        (0, 0) => CritiqueAvailability::Neither,
        (0, _) => CritiqueAvailability::OnlyBaseline,
        (_, 0) => CritiqueAvailability::OnlyVariant,
        (_, _) => CritiqueAvailability::Both,
    };

    if critique == CritiqueAvailability::Both {
        // Likert per-question: union of question sets.
        let qs_a = fetch_likert_questions(variant)?;
        let qs_b = fetch_likert_questions(baseline)?;
        let questions: std::collections::BTreeSet<String> =
            qs_a.iter().chain(qs_b.iter()).cloned().collect();
        for q in &questions {
            let a_samples = fetch_likert_samples(variant, q)?;
            let b_samples = fetch_likert_samples(baseline, q)?;
            if a_samples.is_empty() || b_samples.is_empty() {
                continue;
            }
            match welch_t_test(&a_samples, &b_samples) {
                Ok(outcome) => candidates.push(Finding {
                    kind: FindingKind::LikertQuestion,
                    label: q.clone(),
                    outcome,
                    significant: false,
                }),
                Err(_) => candidates
                    .push(unchanged_placeholder(FindingKind::LikertQuestion, q.clone())),
            }
        }

        // Coded-tag frequencies: union of tag sets.
        let tags_a = fetch_tag_totals(variant)?;
        let tags_b = fetch_tag_totals(baseline)?;
        let tag_names: std::collections::BTreeSet<String> = tags_a
            .iter()
            .map(|t| t.tag.clone())
            .chain(tags_b.iter().map(|t| t.tag.clone()))
            .collect();
        for tag in &tag_names {
            let wins_a = tags_a
                .iter()
                .find(|t| &t.tag == tag)
                .map_or(0, |t| t.count);
            let wins_b = tags_b
                .iter()
                .find(|t| &t.tag == tag)
                .map_or(0, |t| t.count);
            match two_proportion_z_test(wins_a, n_crit_a, wins_b, n_crit_b) {
                Ok(outcome) => candidates.push(Finding {
                    kind: FindingKind::CodedTag,
                    label: tag.clone(),
                    outcome,
                    significant: false,
                }),
                Err(_) => candidates
                    .push(unchanged_placeholder(FindingKind::CodedTag, tag.clone())),
            }
        }
    }

    // --- Apply correction across every family's p-values at once ---
    let p_values: Vec<f64> = candidates
        .iter()
        .map(|f| {
            if f.outcome.p_value.is_nan() {
                1.0
            } else {
                f.outcome.p_value
            }
        })
        .collect();
    let significance_mask = match opts.correction {
        Correction::BenjaminiHochberg => benjamini_hochberg(&p_values, opts.alpha),
        Correction::Bonferroni => bonferroni(&p_values, opts.alpha),
    };
    for (finding, sig) in candidates.iter_mut().zip(significance_mask.iter()) {
        finding.significant = *sig;
    }

    // --- Bucket into flagged / rejected / unchanged ---
    let mut flagged: Vec<Finding> = Vec::new();
    let mut rejected: Vec<Finding> = Vec::new();
    let mut unchanged: Vec<Finding> = Vec::new();
    for finding in candidates {
        if finding.significant {
            flagged.push(finding);
        } else if !finding.outcome.p_value.is_nan() && finding.outcome.p_value < opts.alpha {
            rejected.push(finding);
        } else {
            unchanged.push(finding);
        }
    }

    // Sort:
    //   flagged by |delta| desc (most impactful first) tiebreak by label asc
    //   rejected by raw p asc (nearest-to-significant first)
    //   unchanged by raw p asc (same — reader scans top to identify near-misses)
    flagged.sort_by(|a, b| {
        b.outcome
            .delta
            .abs()
            .partial_cmp(&a.outcome.delta.abs())
            .unwrap_or(core::cmp::Ordering::Equal)
            .then_with(|| a.label.cmp(&b.label))
    });
    rejected.sort_by(|a, b| {
        a.outcome
            .p_value
            .partial_cmp(&b.outcome.p_value)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then_with(|| a.label.cmp(&b.label))
    });
    unchanged.sort_by(|a, b| {
        a.outcome
            .p_value
            .partial_cmp(&b.outcome.p_value)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then_with(|| a.label.cmp(&b.label))
    });

    Ok(CompareResult {
        flagged,
        rejected,
        unchanged,
        only_baseline: paired.only_baseline,
        only_variant: paired.only_variant,
        n_baseline,
        n_variant,
        critique,
        opts: *opts,
    })
}

fn unchanged_placeholder(kind: FindingKind, label: String) -> Finding {
    Finding {
        kind,
        label,
        outcome: TestOutcome {
            mean_a: f64::NAN,
            mean_b: f64::NAN,
            delta: f64::NAN,
            std_err: f64::NAN,
            z_stat: f64::NAN,
            p_value: f64::NAN,
            ci_95_low: None,
            ci_95_high: None,
            n_a: 0,
            n_b: 0,
        },
        significant: false,
    }
}
