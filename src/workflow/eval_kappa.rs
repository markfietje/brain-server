//! Cohen's κ for the eval labeling round (§R11 C5; test-only — this
//! module compiles under `cfg(test)` exclusively and adds no production
//! surface).
//!
//! The labeling discipline: ≥2 human raters label the retained
//! structural/conformance outputs (binary outcome labels read from
//! operator-supplied files in the private repo, by path); the labeling
//! round runs ONLY with those files — absent raters it is the named
//! dataset-readiness gate, never simulated, never fabricated. The
//! agreement statistic is the same binary Cohen's κ the SDK's
//! `pure::calibration` uses, surfaced here as `f64` with NAMED errors
//! (a degenerate input is an error, never NaN).

use std::fmt;

/// Named κ failures. A degenerate division (expected agreement 1) is an
/// error, never NaN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KappaError {
    EmptyRatings,
    LengthMismatch { a: usize, b: usize },
    DegenerateExpectedAgreement,
}

impl fmt::Display for KappaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRatings => write!(f, "kappa: no ratings supplied"),
            Self::LengthMismatch { a, b } => {
                write!(f, "kappa: rating length mismatch ({a} vs {b})")
            }
            Self::DegenerateExpectedAgreement => write!(
                f,
                "kappa: degenerate — both raters constant with the same \
                 marginal (expected agreement 1); κ is undefined"
            ),
        }
    }
}

/// Binary Cohen's κ: (po − pe) / (1 − pe), pe = chance agreement from the
/// raters' own marginals. Same statistic as the SDK's `kappa_units`,
/// returned exactly (f64).
pub(crate) fn cohen_kappa(a: &[bool], b: &[bool]) -> Result<f64, KappaError> {
    if a.is_empty() || b.is_empty() {
        return Err(KappaError::EmptyRatings);
    }
    if a.len() != b.len() {
        return Err(KappaError::LengthMismatch {
            a: a.len(),
            b: b.len(),
        });
    }
    let n = a.len() as f64;
    let observed = a.iter().zip(b).filter(|(x, y)| x == y).count() as f64 / n;
    let a_yes = a.iter().filter(|x| **x).count() as f64 / n;
    let b_yes = b.iter().filter(|y| **y).count() as f64 / n;
    let expected = a_yes * b_yes + (1.0 - a_yes) * (1.0 - b_yes);
    if expected >= 1.0 {
        return Err(KappaError::DegenerateExpectedAgreement);
    }
    Ok((observed - expected) / (1.0 - expected))
}

/// One rater's labels read from a file: `case_id,0|1` per line, `#`
/// comments and blank lines skipped. Malformed content is a named error —
/// never silently dropped (a dropped line would silently inflate
/// agreement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LabelFileError {
    Unreadable(String),
    MalformedLine { line: usize, text: String },
    DuplicateCase { case: String },
}

impl fmt::Display for LabelFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(what) => write!(f, "label file unreadable: {what}"),
            Self::MalformedLine { line, text } => write!(
                f,
                "label file line {line} malformed: {text:?} (expected `case_id,0|1`)"
            ),
            Self::DuplicateCase { case } => {
                write!(f, "label file repeats case {case:?}")
            }
        }
    }
}

pub(crate) fn read_labels(path: &std::path::Path) -> Result<Vec<(String, bool)>, LabelFileError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| LabelFileError::Unreadable(format!("{}: {e}", path.display())))?;
    let mut out: Vec<(String, bool)> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((case, label)) = line.rsplit_once(',') else {
            return Err(LabelFileError::MalformedLine {
                line: i + 1,
                text: line.to_string(),
            });
        };
        let case = case.trim();
        let label = match label.trim() {
            "0" => false,
            "1" => true,
            _ => {
                return Err(LabelFileError::MalformedLine {
                    line: i + 1,
                    text: line.to_string(),
                });
            }
        };
        if case.is_empty() {
            return Err(LabelFileError::MalformedLine {
                line: i + 1,
                text: line.to_string(),
            });
        }
        if out.iter().any(|(c, _)| c == case) {
            return Err(LabelFileError::DuplicateCase {
                case: case.to_string(),
            });
        }
        out.push((case.to_string(), label));
    }
    Ok(out)
}

/// κ over two rater FILES: the case sets must match exactly (a rater who
/// skipped a case is a protocol violation, not an agreement), and the
/// comparison is order-independent.
pub(crate) fn kappa_from_files(a: &std::path::Path, b: &std::path::Path) -> Result<f64, String> {
    let mut la = read_labels(a).map_err(|e| e.to_string())?;
    let mut lb = read_labels(b).map_err(|e| e.to_string())?;
    la.sort();
    lb.sort();
    let ids_a: Vec<&String> = la.iter().map(|(c, _)| c).collect();
    let ids_b: Vec<&String> = lb.iter().map(|(c, _)| c).collect();
    if ids_a != ids_b {
        return Err(format!(
            "kappa: rater case sets differ ({:?} vs {:?}) — every rater \
             labels every retained case",
            ids_a.len(),
            ids_b.len()
        ));
    }
    let va: Vec<bool> = la.into_iter().map(|(_, v)| v).collect();
    let vb: Vec<bool> = lb.into_iter().map(|(_, v)| v).collect();
    cohen_kappa(&va, &vb).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kappa_hand_computable_vectors() {
        // Perfect agreement → exactly 1.
        assert_eq!(
            cohen_kappa(&[true, false, true], &[true, false, true]).unwrap(),
            1.0
        );
        // The standard 2×2 example [[10,2],[3,5]]: po = 0.75,
        // pe = 0.6·0.65 + 0.4·0.35 = 0.53 → κ = 0.22/0.47.
        let a = [
            true, true, true, true, true, true, true, true, true, true, true, true, false, false,
            false, false, false, false, false, false,
        ];
        let b = [
            true, true, true, true, true, true, true, true, true, true, false, false, false, false,
            false, false, false, true, true, true,
        ];
        let k = cohen_kappa(&a, &b).unwrap();
        assert!((k - 0.22 / 0.47).abs() < 1e-12, "κ was {k}");
        // Anti-correlated ratings: agreement below chance → κ ≤ 0 (here −1).
        assert_eq!(
            cohen_kappa(&[true, true, false, false], &[false, false, true, true]).unwrap(),
            -1.0
        );
        // SDK parity: the same inputs through the SDK's integer units
        // (SCALE = 10_000, qa_score's fixed-point factor).
        assert_eq!(
            brain_engine_sdk::pure::calibration::kappa_units(&a, &b),
            Some((0.22f64 / 0.47 * 10_000.0).round() as i32)
        );
    }

    #[test]
    fn kappa_degenerate_inputs_are_named_errors_never_nan() {
        assert_eq!(cohen_kappa(&[], &[]), Err(KappaError::EmptyRatings));
        assert_eq!(
            cohen_kappa(&[true], &[true, false]),
            Err(KappaError::LengthMismatch { a: 1, b: 2 })
        );
        // Both raters constant-same: po = pe = 1 → named error, not NaN —
        // the Err path means no float escapes at all, degenerate or else.
        let r = cohen_kappa(&[true, true, true], &[true, true, true]);
        assert_eq!(r, Err(KappaError::DegenerateExpectedAgreement));
        // The non-degenerate perfect-agreement path returns an exact,
        // finite 1.0 (both marginals present, so pe < 1).
        assert_eq!(cohen_kappa(&[true, false], &[true, false]).unwrap(), 1.0);
    }

    #[test]
    fn label_files_read_by_path_with_named_malformations() {
        let dir = tempfile::tempdir().unwrap();
        let path = |name: &str| dir.path().join(name);
        std::fs::write(
            path("a.csv"),
            "# rater A\nhappy-battery,1\nfail-intake,0\nhappy-thermal,1\n",
        )
        .unwrap();
        let labels = read_labels(&path("a.csv")).unwrap();
        assert_eq!(
            labels,
            vec![
                ("happy-battery".to_string(), true),
                ("fail-intake".to_string(), false),
                ("happy-thermal".to_string(), true)
            ]
        );
        // A rater who skipped a case is a protocol violation.
        std::fs::write(path("b.csv"), "happy-battery,1\nhappy-thermal,0\n").unwrap();
        let err = kappa_from_files(&path("a.csv"), &path("b.csv")).unwrap_err();
        assert!(err.contains("case sets differ"), "{err}");
        // Malformed label → named, never dropped.
        std::fs::write(path("c.csv"), "happy-battery,2\n").unwrap();
        assert!(matches!(
            read_labels(&path("c.csv")),
            Err(LabelFileError::MalformedLine { line: 1, .. })
        ));
        // Order-independent agreement on the same set.
        std::fs::write(
            path("b2.csv"),
            "happy-thermal,1\nhappy-battery,1\nfail-intake,0\n",
        )
        .unwrap();
        assert_eq!(
            kappa_from_files(&path("a.csv"), &path("b2.csv")).unwrap(),
            1.0
        );
    }

    #[test]
    fn labeling_round_runs_only_with_operator_rater_files() {
        // The labeling round is an OPERATOR input: two rater files over the
        // retained structural outputs, supplied via the declared test-time
        // paths (the R10 pack-reader pattern). Absent files this is the
        // NAMED dataset-readiness gate — the run is recorded, never
        // simulated, never fabricated.
        let (Some(path_a), Some(path_b)) = (
            std::env::var("GDL_R11_RATER_A_PATH").ok(),
            std::env::var("GDL_R11_RATER_B_PATH").ok(),
        ) else {
            println!(
                "# DATASET-READINESS GATE (named, OPEN): the κ labeling round did NOT run — \
                 no operator rater files were supplied (GDL_R11_RATER_A_PATH / \
                 GDL_R11_RATER_B_PATH). Inter-rater agreement for the structural outputs \
                 does not exist yet; pilot readiness cannot cite κ."
            );
            return;
        };
        let kappa = kappa_from_files(
            &std::path::PathBuf::from(path_a),
            &std::path::PathBuf::from(path_b),
        )
        .unwrap_or_else(|e| panic!("operator rater files present but unusable: {e}"));
        println!(
            "# LABELING ROUND RUN: binary outcome κ = {kappa:.4} over the \
             operator rater files (readiness bar for the pilot pack: ≥ 0.70)"
        );
    }
}
