use crate::state::InterviewEnvelope;
use sha2::{Digest, Sha256};
fn hash(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}
pub struct Page {
    pub items: Vec<String>,
    pub cursor: Option<String>,
    pub digest: String,
}
pub fn summary(env: &InterviewEnvelope) -> Page {
    let s = serde_json::to_string(env).unwrap();
    let d = hash(&s);
    Page {
        items: vec![s.clone()],
        cursor: None,
        digest: d,
    }
}
pub fn pending(env: &InterviewEnvelope) -> Page {
    let items: Vec<String> = env
        .state
        .rounds
        .iter()
        .filter(|r| r.lifecycle == "answered")
        .map(|r| r.id.clone())
        .collect();
    let j = serde_json::to_string(&items).unwrap();
    Page {
        items,
        cursor: None,
        digest: hash(&j),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::*;
    #[test]
    fn deterministic() {
        let e = initialize_context(3000, "native");
        let a = summary(&e);
        let b = summary(&e);
        assert_eq!(a.digest, b.digest);
    }
}
