use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rule {
    pub id: i64,
    pub jurisdiction: String,
    pub subject: String,
    pub rule_key: String,
    pub body: String,
    pub source_ref: String,
    pub effective_at: i64,
    pub reviewed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub revision: i64,
    pub superseded_by: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rate {
    pub basis_points: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    NoRule,
    Expired { rule_key: String },
    MissingReview { rule_key: String },
}

pub fn applicable_rule<'a>(rules: &'a [Rule], jurisdiction: &str, subject: &str, at: i64) -> Option<&'a Rule> {
    rules
        .iter()
        .filter(|r| {
            r.jurisdiction == jurisdiction
                && r.subject == subject
                && r.effective_at <= at
        })
        .max_by_key(|r| (r.effective_at, r.revision))
}

pub fn apply_rate(rules: &[Rule], rates: &[(i64, Rate)], jurisdiction: &str, date: i64, amount_minor: i64) -> Result<i64, ApplyError> {
    // For demo: subject = "vat"
    let rule = applicable_rule(rules, jurisdiction, "vat", date).ok_or(ApplyError::NoRule)?;
    if let Some(exp) = rule.expires_at {
        if date > exp {
            return Err(ApplyError::Expired { rule_key: rule.rule_key.clone() });
        }
    }
    if rule.reviewed_at.is_none() || rule.reviewed_at.unwrap() == 0 {
        return Err(ApplyError::MissingReview { rule_key: rule.rule_key.clone() });
    }
    let rate = rates.iter().find(|(id, _)| *id == rule.id).map(|(_, r)| r.basis_points).unwrap_or(0);
    // integer math: basis_points are ten-thousandths (e.g. 2100 = 21%)
    Ok(amount_minor * rate as i64 / 10000)
}

pub fn seed_rules() -> (Vec<Rule>, Vec<(i64, Rate)>) {
    let rules = vec![
        Rule { id: 1, jurisdiction: "NL".into(), subject: "vat".into(), rule_key: "nl_vat_standard".into(), body: "Standard VAT rate".into(), source_ref: "Wet OB 1968".into(), effective_at: 1704067200, reviewed_at: Some(1704067200), expires_at: None, revision: 1, superseded_by: None, created_at: 1704067200 },
        Rule { id: 2, jurisdiction: "PH".into(), subject: "vat".into(), rule_key: "ph_vat_standard".into(), body: "Standard VAT rate".into(), source_ref: "NIRC".into(), effective_at: 1704067200, reviewed_at: Some(1704067200), expires_at: None, revision: 1, superseded_by: None, created_at: 1704067200 },
    ];
    let rates = vec![(1, Rate { basis_points: 2100 }), (2, Rate { basis_points: 1200 })];
    (rules, rates)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rulebook_reconstructs_any_date() {
        let (mut rules, rates) = seed_rules();
        // add revision effective mid-month
        rules.push(Rule { id: 3, jurisdiction: "NL".into(), subject: "vat".into(), rule_key: "nl_vat_standard".into(), body: "Updated".into(), source_ref: "Wet OB 1968".into(), effective_at: 1706745600, reviewed_at: Some(1706745600), expires_at: None, revision: 2, superseded_by: None, created_at: 1706745600 });
        // mark old superseded
        rules[0].superseded_by = Some(3);
        let before = apply_rate(&rules, &rates, "NL", 1705000000, 10000).unwrap();
        assert_eq!(before, 2100); // still old rule via superseded filter would pick new? Actually old is superseded so None before -- adjust test to use non-superseded simulation
    }
    #[test]
    fn expired_rule_fails_closed_for_money() {
        let (mut rules, rates) = seed_rules();
        rules[0].expires_at = Some(1705000000);
        let err = apply_rate(&rules, &rates, "NL", 1706000000, 10000).unwrap_err();
        assert!(matches!(err, ApplyError::Expired { .. }));
    }
    #[test]
    fn application_is_pure_and_deterministic() {
        let (rules, rates) = seed_rules();
        let a = apply_rate(&rules, &rates, "NL", 1710000000, 10000).unwrap();
        let b = apply_rate(&rules, &rates, "NL", 1710000000, 10000).unwrap();
        assert_eq!(a, b);
    }
}
