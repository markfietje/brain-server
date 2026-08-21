pub const MAX_STEPS_PER_TURN: u32 = 24;
pub const MAX_STEPS_CEILING: u32 = 1_000;
pub const MAX_STEERING_QUEUE_SIZE: usize = 100;
pub const MAX_PAUSE_CONTINUATIONS: u32 = 3;

pub fn clamp_max_steps(v: u32) -> u32 {
    v.clamp(1, MAX_STEPS_CEILING)
}

pub fn resolve_max_steps(env_val: Option<u32>) -> u32 {
    match env_val {
        Some(v) if v > MAX_STEPS_CEILING => {
            eprintln!("warning: max_steps {v} exceeds ceiling, clamped to {MAX_STEPS_CEILING}");
            MAX_STEPS_CEILING
        }
        Some(0) => {
            eprintln!("warning: max_steps 0 invalid, using default {MAX_STEPS_PER_TURN}");
            MAX_STEPS_PER_TURN
        }
        Some(v) => v,
        None => MAX_STEPS_PER_TURN,
    }
}

pub const fn should_warn_at_iteration_threshold(current: u32, max: u32) -> bool {
    if max < 5 {
        return false;
    }
    current >= max.saturating_mul(4) / 5
}

pub fn handoff_steering_text(remaining: u32) -> String {
    format!(
        "Approaching step budget: {remaining} steps remaining. Consider summarizing findings and preparing handoff."
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Completed,
    Aborted,
    Paused,
}

#[derive(Debug, Clone)]
pub struct Step {
    pub id: u64,
    pub action: String,
    pub evidence_refs: Vec<String>,
    pub verdict: Option<Verdict>,
}

#[derive(Debug, Clone)]
pub struct Turn {
    pub steps: Vec<Step>,
    pub closed: bool,
}

impl Turn {
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            closed: false,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

impl Default for Turn {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct SteeringInbox {
    queue: std::collections::VecDeque<String>,
}

impl SteeringInbox {
    pub fn new() -> Self {
        Self {
            queue: std::collections::VecDeque::new(),
        }
    }
    pub fn push(&mut self, msg: String) {
        if self.queue.len() >= MAX_STEERING_QUEUE_SIZE {
            self.queue.pop_front();
        }
        self.queue.push_back(msg);
    }
    pub fn drain_at_boundary(&mut self) -> Vec<String> {
        self.queue.drain(..).collect()
    }
    pub fn len(&self) -> usize {
        self.queue.len()
    }
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl Default for SteeringInbox {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct RunState {
    pub turn: Turn,
    pub inbox: SteeringInbox,
    pub max_steps: u32,
    pub pause_continuations: u32,
    pub step_count: u32,
}

impl RunState {
    pub fn new(max_steps: u32) -> Self {
        Self {
            turn: Turn::new(),
            inbox: SteeringInbox::new(),
            max_steps: clamp_max_steps(max_steps),
            pause_continuations: 0,
            step_count: 0,
        }
    }
    pub fn should_warn(&self) -> bool {
        should_warn_at_iteration_threshold(self.step_count, self.max_steps)
    }
    pub fn can_pause_again(&self) -> bool {
        self.pause_continuations < MAX_PAUSE_CONTINUATIONS
    }
    pub fn record_step(&mut self, step: Step) -> Result<(), &'static str> {
        if self.step_count >= self.max_steps {
            return Err("step budget exhausted");
        }
        self.step_count += 1;
        self.turn.steps.push(step);
        Ok(())
    }
    pub fn checkpoint(&self, interrupted: bool) -> Option<Step> {
        if interrupted {
            Some(Step {
                id: self.step_count as u64 + 1,
                action: "checkpoint".into(),
                evidence_refs: vec![],
                verdict: Some(Verdict::Aborted),
            })
        } else {
            None
        }
    }
}
