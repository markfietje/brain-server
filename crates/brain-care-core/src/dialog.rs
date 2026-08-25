//! The care dialog: an interview-core interview bound to a care worktype.
//! Ambiguity scoring, drafts, and revision repair are the interview's own
//! machinery, re-exported verbatim — a care dialog introduces no new
//! decision primitive.

use brain_interview_core::draft::DraftStore;

/// The run kinds that may open a care dialog. Closed vocabulary — anything
/// else denies loudly.
pub const CARE_KINDS: [&str; 2] = ["care_inquiry", "account"];

#[derive(Debug)]
pub struct CareDialog {
    kind: String,
    drafts: DraftStore,
}

impl CareDialog {
    pub fn open(kind: &str) -> Result<Self, String> {
        if !CARE_KINDS.contains(&kind) {
            return Err(format!("not_a_care_worktype: {kind}"));
        }
        Ok(Self {
            kind: kind.to_string(),
            drafts: DraftStore::default(),
        })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn drafts(&mut self) -> &mut DraftStore {
        &mut self.drafts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_interview_core::ambiguity::{
        clamp_reported, compute_ambiguity_floor, weighted_ambiguity_units,
    };
    use brain_interview_core::repair::repair_revision_conflict;

    #[test]
    fn care_core_reuses_interview_machinery_zero_new_concepts() {
        // A care dialog is just an interview with a worktype binding.
        let mut d = CareDialog::open("care_inquiry").expect("opens");
        assert_eq!(d.kind(), "care_inquiry");
        assert!(CareDialog::open("astrology_reading").is_err());
        // Ambiguity floor math IS interview-core's math, unchanged.
        let units = weighted_ambiguity_units(&[0.5_f64, 0.6, 0.7], false).expect("units");
        let floor = compute_ambiguity_floor(1, 1, 50, 100);
        let (clamped, clamped_by_floor) = clamp_reported(units, floor.floor_units);
        assert_eq!(clamped, u32::max(units, floor.floor_units));
        assert_eq!(clamped_by_floor, floor.floor_units > units);
        // Drafts ride the same store + revision-conflict repair text.
        d.drafts()
            .create("dr-1", "summary of inquiry".to_string(), 10)
            .expect("create");
        let conflict = repair_revision_conflict(1, 2);
        assert!(!conflict.is_empty());
    }
}
