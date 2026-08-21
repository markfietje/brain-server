use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorBreakdown {
    pub disputed_count: u32,
    pub unscored_active: u32,
    pub auto_ratio_units: u32,
    pub floor_units: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Milestone { Ready, Initial, Progress, Refined }

pub fn score_to_units(score: f64) -> Result<u32, String> {
    if !score.is_finite() || !(0.0..=1.0).contains(&score) { return Err("DI_INVALID_ARGUMENT".into()); }
    let s = format!("{:.4}", score);
    let parts: Vec<&str> = s.split('.').collect();
    let int_part: u32 = parts[0].parse().unwrap();
    let frac: u32 = parts[1].parse().unwrap();
    Ok(int_part*10000 + frac)
}

pub fn weighted_ambiguity_units(scores: &[f64], is_brownfield: bool) -> Result<u32, String> {
    let units: Vec<u32> = scores.iter().map(|s| score_to_units(*s)).collect::<Result<_,_>>()?;
    let numerator = if is_brownfield {
        if units.len()!=4 { return Err("DI_INVALID_ARGUMENT".into()); }
        units[0]*35 + units[1]*25 + units[2]*25 + units[3]*15
    } else {
        if units.len()!=3 { return Err("DI_INVALID_ARGUMENT".into()); }
        units[0]*40 + units[1]*30 + units[2]*30
    };
    let weighted = (numerator + 50)/100;
    Ok(10000u32.saturating_sub(weighted))
}

pub fn compute_ambiguity_floor(disputed: u32, unscored: u32, auto_scored: u32, scored: u32) -> FloorBreakdown {
    let auto_ratio = if scored==0 {0} else { std::cmp::min(10000, (auto_scored*10000)/scored) };
    let floor = std::cmp::min(10000, disputed*1000 + unscored*500 + auto_ratio/20);
    FloorBreakdown{ disputed_count: disputed, unscored_active: unscored, auto_ratio_units: auto_ratio, floor_units: floor }
}

pub fn clamp_reported(reported_units: u32, floor_units: u32) -> (u32,bool) {
    let eff = reported_units.min(10000);
    if floor_units > eff { (floor_units.min(10000), true) } else { (eff,false) }
}

pub fn derive_milestone(effective: u32, threshold: u32) -> Milestone {
    if effective <= threshold { Milestone::Ready }
    else if effective > 6000 { Milestone::Initial }
    else if effective > 3000 { Milestone::Progress }
    else { Milestone::Refined }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn contradiction_raises_ambiguity() {
        let a = weighted_ambiguity_units(&[0.9,0.9,0.9], false).unwrap();
        let b = weighted_ambiguity_units(&[0.3,0.9,0.9], false).unwrap();
        assert!(b > a);
    }
    #[test] fn floor_blocks_until_superseded() {
        let f = compute_ambiguity_floor(1,0,0,4);
        assert_eq!(f.floor_units, 1000);
        let (eff, clamped) = clamp_reported(200, f.floor_units);
        assert!(clamped && eff==1000);
    }
}
