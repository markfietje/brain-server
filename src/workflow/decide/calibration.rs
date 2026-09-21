//! The calibration math — the pure port of the laya confidence/ECE/
//! temperature contract (LAYA_RUST_PORT §0.3/§2.4) plus the steward
//! integer-units pin: `f32` exists ONLY in this file (the conversion
//! boundary); every stored or compared score is `u32` ten-thousandths
//! (0..=10000), and anything off the ≤4-decimal grid refuses instead of
//! clamping.

use super::sequence::QType;

/// Confidence = 1 − H/ln(k) over the probability vector, clipped 0..1;
/// `k < 2` (a coin or a single option) is perfectly confident by the
/// source's convention.
pub(crate) fn confidence_from_probs(p: &[f32], k: usize) -> f32 {
    if k < 2 || p.is_empty() {
        return 1.0;
    }
    let ln_k = (k as f32).ln();
    let entropy: f32 = p
        .iter()
        .filter(|pi| **pi > 0.0)
        .map(|pi| -pi * pi.ln())
        .sum();
    (1.0 - entropy / ln_k).clamp(0.0, 1.0)
}

/// The temperature bucket key: `{type}:{2|3-5|6-10|11+}` — one fitted
/// temperature per option-count band per question type.
pub(crate) fn temp_bucket(qtype: QType, k: usize) -> String {
    let band = match k {
        0 | 1 | 2 => "2",
        3..=5 => "3-5",
        6..=10 => "6-10",
        _ => "11+",
    };
    format!("{}:{}", qtype.as_str(), band)
}

/// Expected calibration error over equally-spaced bins; empty input is
/// NaN (matching the source's `float("nan")` — the caller must treat it
/// as "no measure", never as a score).
pub(crate) fn ece_score(conf: &[f32], correct: &[bool], bins: usize) -> f32 {
    if conf.len() != correct.len() {
        return f32::NAN;
    }
    if conf.is_empty() {
        return f32::NAN;
    }
    if bins == 0 {
        return f32::NAN;
    }
    let mut bin_count = vec![0usize; bins];
    let mut bin_correct = vec![0usize; bins];
    let mut bin_conf = vec![0.0f32; bins];
    for (c, ok) in conf.iter().zip(correct.iter()) {
        let raw = (c * bins as f32).floor() as usize;
        let b = raw.min(bins - 1);
        bin_count[b] += 1;
        bin_conf[b] += *c;
        if *ok {
            bin_correct[b] += 1;
        }
    }
    let total = conf.len() as f32;
    let mut ece = 0.0f32;
    for b in 0..bins {
        if bin_count[b] == 0 {
            continue;
        }
        let acc = bin_correct[b] as f32 / bin_count[b] as f32;
        let avg_conf = bin_conf[b] / bin_count[b] as f32;
        ece += (bin_count[b] as f32 / total) * (acc - avg_conf).abs();
    }
    ece
}

/// Numerically-stable in-place softmax (the post-step boundary helper).
pub(crate) fn softmax_inplace(z: &mut [f32]) {
    let Some(max) = z.iter().copied().max_by(|a, b| a.total_cmp(b)) else {
        return;
    };
    let mut sum = 0.0f32;
    for v in z.iter_mut() {
        *v = (*v - max).exp();
        sum += *v;
    }
    if sum > 0.0 {
        for v in z.iter_mut() {
            *v /= sum;
        }
    }
}

/// The steward grid: a score becomes `u32` ten-thousandths ONLY when it
/// sits in 0..=1 with at most 4 decimal places (the shortest round-trip
/// decimal decides — no epsilon fudging). Off-grid, out-of-range, and
/// NaN all refuse (`None`); the caller escalates, never clamps.
pub(crate) fn score_to_units(s: f32) -> Option<u32> {
    if !s.is_finite() || !(0.0..=1.0).contains(&s) {
        return None;
    }
    let text = format!("{s}");
    let decimals = text
        .rsplit_once('.')
        .map(|(_, frac)| frac.len())
        .unwrap_or(0);
    if decimals > 4 {
        return None;
    }
    let scaled = s * 10_000.0;
    let units = scaled.round();
    if units < 0.0 || units > 10_000.0 {
        return None;
    }
    Some(units as u32)
}

/// The display-only inverse (never a store path — units are the law).
pub(crate) fn units_to_f32(u: u32) -> f32 {
    u as f32 / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── confidence (the entropy vectors) ────────────────────────────────
    #[test]
    fn confidence_uniform_is_zero() {
        let p = [0.25f32, 0.25, 0.25, 0.25];
        let conf = confidence_from_probs(&p, 4);
        assert!(
            conf < 0.001,
            "uniform over 4 is maximally unconfident: {conf}"
        );
    }
    #[test]
    fn confidence_one_hot_is_one() {
        let p = [0.0f32, 0.0, 1.0, 0.0];
        assert!((confidence_from_probs(&p, 4) - 1.0).abs() < 1e-6);
    }
    #[test]
    fn confidence_k_of_one_is_one() {
        assert_eq!(confidence_from_probs(&[1.0], 1), 1.0);
    }
    #[test]
    fn confidence_k_below_two_is_one() {
        assert_eq!(confidence_from_probs(&[], 0), 1.0);
    }
    #[test]
    fn confidence_peaked_beats_flat() {
        let peaked = [0.9f32, 0.05, 0.05];
        let flat = [0.4f32, 0.3, 0.3];
        assert!(confidence_from_probs(&peaked, 3) > confidence_from_probs(&flat, 3));
    }

    // ── temp_bucket (the band edges) ─────────────────────────────────────
    #[test]
    fn temp_bucket_edges_pin_the_bands() {
        assert_eq!(temp_bucket(QType::Choice, 2), "choice:2");
        assert_eq!(temp_bucket(QType::Choice, 3), "choice:3-5");
        assert_eq!(temp_bucket(QType::Choice, 5), "choice:3-5");
        assert_eq!(temp_bucket(QType::Score, 6), "score:6-10");
        assert_eq!(temp_bucket(QType::Score, 10), "score:6-10");
        assert_eq!(temp_bucket(QType::Noul, 11), "noul:11+");
        assert_eq!(temp_bucket(QType::Noul, 77), "noul:11+");
    }

    // ── ECE (hand-computed) ─────────────────────────────────────────────
    #[test]
    fn ece_empty_is_nan_never_zero() {
        assert!(ece_score(&[], &[], 15).is_nan());
        assert!(ece_score(&[0.5], &[true], 0).is_nan());
    }
    #[test]
    fn ece_length_mismatch_is_nan() {
        assert!(ece_score(&[0.5], &[], 15).is_nan());
    }
    #[test]
    fn ece_hand_computed_two_bin_case() {
        // Two confident-correct (0.9, T) and two confident-wrong (0.9, F):
        // one bin holds all four → |0.5 - 0.9| = 0.4.
        let conf = [0.9f32, 0.9, 0.9, 0.9];
        let correct = [true, true, false, false];
        let ece = ece_score(&conf, &correct, 10);
        assert!((ece - 0.4).abs() < 1e-5, "hand ECE 0.4, got {ece}");
    }
    #[test]
    fn ece_perfect_calibration_is_near_zero() {
        let conf = [1.0f32, 0.0];
        let correct = [true, false];
        let ece = ece_score(&conf, &correct, 15);
        assert!(ece < 1e-6, "perfect calibration scores ~0, got {ece}");
    }

    // ── softmax ─────────────────────────────────────────────────────────
    #[test]
    fn softmax_is_stable_and_sums_to_one() {
        let mut z = [1000.0f32, 1001.0, 999.0];
        softmax_inplace(&mut z);
        let sum: f32 = z.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(z[1] > z[0] && z[0] > z[2]);
    }
    #[test]
    fn softmax_of_empty_is_a_no_op() {
        let mut z: Vec<f32> = Vec::new();
        softmax_inplace(&mut z);
        assert!(z.is_empty());
    }

    // ── score_to_units (the steward grid) ────────────────────────────────
    #[test]
    fn score_to_units_grid_rejects() {
        assert_eq!(score_to_units(0.21000001), None, "8dp off-grid refuses");
        assert_eq!(score_to_units(1.00001), None, "out of range refuses");
        assert_eq!(score_to_units(-0.1), None, "negative refuses");
        assert_eq!(score_to_units(1.1), None, "over one refuses");
        assert_eq!(score_to_units(f32::NAN), None, "NaN refuses");
        assert_eq!(score_to_units(f32::INFINITY), None, "inf refuses");
    }
    #[test]
    fn score_to_units_accepts_the_grid() {
        assert_eq!(score_to_units(0.0), Some(0));
        assert_eq!(score_to_units(1.0), Some(10_000));
        assert_eq!(score_to_units(0.21), Some(2_100));
        assert_eq!(score_to_units(0.75), Some(7_500));
        assert_eq!(score_to_units(0.85), Some(8_500));
        assert_eq!(score_to_units(0.0001), Some(1));
    }
    #[test]
    fn units_round_trip() {
        let units = score_to_units(0.4079).unwrap();
        assert_eq!(units, 4_079);
        assert!((units_to_f32(units) - 0.4079).abs() < 1e-6);
        assert_eq!(units_to_f32(0), 0.0);
    }

    // ── the source scan (the rubric pin, enforced at the file boundary) ──
    #[test]
    fn no_f32_in_decide_math_outside_boundary() {
        // calibration.rs (this file) is THE conversion boundary this round;
        // lang.rs/router.rs may carry the detection-fraction signatures only
        // (their f32 is display measurement, never a stored score); the
        // transport-free modules carry none at all. inference.rs is the
        // Phase 1 file and does not exist yet.
        for (file, allowed) in [
            ("mod.rs", false),
            ("sequence.rs", false),
            ("presets.rs", false),
            ("lang.rs", true),
            ("router.rs", true),
        ] {
            let source = match file {
                "mod.rs" => include_str!("mod.rs"),
                "sequence.rs" => include_str!("sequence.rs"),
                "presets.rs" => include_str!("presets.rs"),
                "lang.rs" => include_str!("lang.rs"),
                "router.rs" => include_str!("router.rs"),
                _ => unreachable!("test table"),
            };
            if !allowed {
                assert!(
                    !source.contains("f32"),
                    "{file} must carry no f32 — the units boundary is calibration.rs"
                );
            }
        }
        // calibration.rs itself: units only leave through score_to_units.
        let me = include_str!("calibration.rs");
        assert!(me.contains("pub(crate) fn score_to_units(s: f32) -> Option<u32>"));
        assert!(me.contains("pub(crate) fn units_to_f32(u: u32) -> f32"));
    }
}
