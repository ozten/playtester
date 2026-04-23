//! Integration tests for the Phase 6 compare markdown reporter.
//!
//! Builds synthetic `CompareResult` values (rather than going
//! through `run_compare`) so each test pins exactly one rendering
//! concern.

use playtest_metrics::{
    CompareOpts, CompareResult, Correction, CritiqueAvailability, Finding, FindingKind,
    MarkdownBuilder, MetricKey, TestOutcome, write_compare_report,
};

fn outcome(mean_a: f64, mean_b: f64, p: f64, n_a: u64, n_b: u64) -> TestOutcome {
    TestOutcome {
        mean_a,
        mean_b,
        delta: mean_a - mean_b,
        std_err: 0.5,
        z_stat: 2.0,
        p_value: p,
        ci_95_low: Some(mean_a - mean_b - 0.98),
        ci_95_high: Some(mean_a - mean_b + 0.98),
        n_a,
        n_b,
    }
}

fn finding(kind: FindingKind, label: &str, out: TestOutcome, significant: bool) -> Finding {
    Finding {
        kind,
        label: label.into(),
        outcome: out,
        significant,
    }
}

fn base_result() -> CompareResult {
    CompareResult {
        n_baseline: 100,
        n_variant: 100,
        critique: CritiqueAvailability::Neither,
        opts: CompareOpts::default(),
        ..Default::default()
    }
}

// -----------------------------------------------------------------

#[test]
fn empty_result_still_emits_heading_and_no_data_note() {
    let mut md = MarkdownBuilder::new();
    let result = base_result();
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("## Compare"));
    assert!(out.contains("*No comparable data.*"));
}

#[test]
fn header_line_shows_sample_sizes_correction_and_alpha() {
    let mut md = MarkdownBuilder::new();
    let result = base_result();
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("baseline = **100**"));
    assert!(out.contains("variant = **100**"));
    assert!(out.contains("**BH**"));
    assert!(out.contains("α = **0.05**"));
}

#[test]
fn flagged_numeric_findings_split_by_delta_sign() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    // variant larger for "metric_up", smaller for "metric_down"
    result.flagged.push(finding(
        FindingKind::NumericMetric,
        "metric_up",
        outcome(12.0, 10.0, 0.001, 100, 100),
        true,
    ));
    result.flagged.push(finding(
        FindingKind::NumericMetric,
        "metric_down",
        outcome(8.0, 10.0, 0.001, 100, 100),
        true,
    ));
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("### Flagged: variant > baseline"));
    assert!(out.contains("### Flagged: variant < baseline"));
    // Each section contains the right metric name.
    let up_idx = out.find("variant > baseline").unwrap();
    let down_idx = out.find("variant < baseline").unwrap();
    let up_section = &out[up_idx..down_idx];
    let down_section = &out[down_idx..];
    assert!(up_section.contains("metric_up"));
    assert!(!up_section.contains("metric_down"));
    assert!(down_section.contains("metric_down"));
}

#[test]
fn agent_table_renders_win_rate_as_percentage() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    result.flagged.push(finding(
        FindingKind::AgentWinRate,
        "random",
        outcome(0.8, 0.5, 0.001, 100, 100),
        true,
    ));
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("Per-agent win-rate deltas"));
    assert!(out.contains("80.0%"));
    assert!(out.contains("50.0%"));
    assert!(out.contains("+30.0pp"));
}

#[test]
fn critique_only_baseline_renders_skip_note() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    result.critique = CritiqueAvailability::OnlyBaseline;
    // Non-empty mechanical side so the report doesn't short-circuit.
    result.flagged.push(finding(
        FindingKind::NumericMetric,
        "pegs",
        outcome(15.0, 12.0, 0.01, 100, 100),
        true,
    ));
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("### Subjective deltas"));
    assert!(out.contains("Baseline has critique data but variant does not"));
}

#[test]
fn critique_both_sides_renders_likert_and_tag_tables() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    result.critique = CritiqueAvailability::Both;
    result.flagged.push(finding(
        FindingKind::LikertQuestion,
        "agency",
        outcome(2.0, 4.0, 0.001, 100, 100),
        true,
    ));
    result.flagged.push(finding(
        FindingKind::CodedTag,
        "forced_sacrifice",
        outcome(0.3, 0.05, 0.0005, 100, 100),
        true,
    ));
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("### Subjective deltas"));
    assert!(out.contains("Likert"));
    assert!(out.contains("agency"));
    assert!(out.contains("Coded tags"));
    assert!(out.contains("forced_sacrifice"));
}

#[test]
fn sample_size_imbalance_emits_warning_banner() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    result.n_baseline = 100;
    result.n_variant = 1000;
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("**Warning:**"));
    assert!(out.contains("10.0×"));
}

#[test]
fn balanced_cohorts_emit_no_warning_banner() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    result.n_baseline = 100;
    result.n_variant = 150; // ratio 1.5, below threshold
    // non-empty mechanical to avoid short-circuit
    result.unchanged.push(finding(
        FindingKind::NumericMetric,
        "pegs",
        outcome(12.0, 12.1, 0.5, 150, 100),
        false,
    ));
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(!out.contains("**Warning:**"));
}

#[test]
fn rejected_bucket_folds_to_top_five() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    for i in 0..8 {
        result.rejected.push(finding(
            FindingKind::NumericMetric,
            &format!("metric_{i}"),
            outcome(12.0 + f64::from(i) * 0.01, 12.0, 0.03, 100, 100),
            false,
        ));
    }
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("### Rejected (correction noise)"));
    // Top 5 shown; "... plus 3 more" line.
    assert!(out.contains("plus 3 more rejected findings"));
}

#[test]
fn only_baseline_and_only_variant_metrics_render_as_bullet_list() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    result.only_baseline.push(MetricKey {
        name: "old_metric".into(),
        player: None,
        tag: None,
    });
    result.only_variant.push(MetricKey {
        name: "new_metric".into(),
        player: None,
        tag: None,
    });
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("Metrics present in only one cohort"));
    assert!(out.contains("**Only in baseline**"));
    assert!(out.contains("old_metric"));
    assert!(out.contains("**Only in variant**"));
    assert!(out.contains("new_metric"));
}

#[test]
fn p_value_below_point_001_renders_as_less_than() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    result.flagged.push(finding(
        FindingKind::NumericMetric,
        "big_signal",
        outcome(100.0, 50.0, 0.000_000_1, 100, 100),
        true,
    ));
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("< 0.001"));
}

#[test]
fn correction_bonferroni_renders_in_header() {
    let mut md = MarkdownBuilder::new();
    let mut result = base_result();
    result.opts = CompareOpts {
        alpha: 0.01,
        correction: Correction::Bonferroni,
    };
    write_compare_report(&mut md, &result);
    let out = md.into_string();
    assert!(out.contains("**Bonferroni**"));
    assert!(out.contains("α = **0.01**"));
}

#[test]
fn rendering_is_deterministic_byte_for_byte() {
    let mut result = base_result();
    result.flagged.push(finding(
        FindingKind::NumericMetric,
        "pegs",
        outcome(15.0, 12.0, 0.0001, 100, 100),
        true,
    ));
    let a = {
        let mut md = MarkdownBuilder::new();
        write_compare_report(&mut md, &result);
        md.into_string()
    };
    let b = {
        let mut md = MarkdownBuilder::new();
        write_compare_report(&mut md, &result);
        md.into_string()
    };
    assert_eq!(a, b);
}
