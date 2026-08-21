pub const MAX_PARALLEL_TASKS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskKind {
    Read,
    Mutate,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub kind: TaskKind,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaMode {
    Strict,
}

#[derive(Debug)]
pub struct DelegationPlan {
    pub tasks: Vec<Task>,
}

impl DelegationPlan {
    pub fn validate(&self) -> Result<(), String> {
        let parallel_reads = self.tasks.iter().filter(|t| t.kind == TaskKind::Read).count();
        if parallel_reads > MAX_PARALLEL_TASKS {
            return Err(format!("too many parallel reads: {parallel_reads} > {MAX_PARALLEL_TASKS}"));
        }
        let mutations = self.tasks.iter().filter(|t| t.kind == TaskKind::Mutate).count();
        if mutations > 1 {
            return Err("mutations must run serially, one per step".into());
        }
        Ok(())
    }

    pub fn execution_order(&self) -> Vec<Vec<Task>> {
        let reads: Vec<Task> = self.tasks.iter().filter(|t| t.kind == TaskKind::Read).cloned().collect();
        let mutates: Vec<Task> = self.tasks.iter().filter(|t| t.kind == TaskKind::Mutate).cloned().collect();
        let mut batches = Vec::new();
        for chunk in reads.chunks(MAX_PARALLEL_TASKS) {
            batches.push(chunk.to_vec());
        }
        for m in mutates {
            batches.push(vec![m]);
        }
        if batches.is_empty() && !self.tasks.is_empty() {
            batches.push(self.tasks.clone());
        }
        batches
    }
}

pub fn validate_schema_strict(json: &str, required_fields: &[&str]) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("invalid json: {e}"))?;
    for f in required_fields {
        if v.get(f).is_none() {
            return Err(format!("missing required field: {f}"));
        }
    }
    Ok(())
}
