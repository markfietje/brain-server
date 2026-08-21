#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Note,
    Concern,
    Blocker,
}

#[derive(Debug, Clone)]
pub struct Advice {
    pub verdict: Verdict,
    pub message: String,
}

pub struct Advisor {
    consecutive_failures: u32,
    disabled: bool,
    seen: std::collections::HashSet<String>,
    rate_limit: usize,
}

impl Advisor {
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
            disabled: false,
            seen: std::collections::HashSet::new(),
            rate_limit: 10,
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn advise(&mut self, input: &str, trivial: bool) -> Option<Advice> {
        if self.disabled || trivial {
            return None;
        }
        if self.seen.contains(input) {
            return None;
        }
        if self.seen.len() >= self.rate_limit {
            return None;
        }
        self.seen.insert(input.to_string());
        Some(Advice {
            verdict: Verdict::Note,
            message: input.to_string(),
        })
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= 3 {
            self.disabled = true;
        }
    }

    pub fn blocker_pauses(&self, advice: &Advice) -> bool {
        advice.verdict == Verdict::Blocker
    }
}

impl Default for Advisor {
    fn default() -> Self {
        Self::new()
    }
}
