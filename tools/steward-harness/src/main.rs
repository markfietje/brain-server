use std::io::{BufRead, BufReader};
fn main() {
    let stdin = std::io::stdin();
    for line in BufReader::new(stdin).lines().map_while(Result::ok) {
        if line.trim().is_empty() { continue; }
        let v: serde_json::Value = serde_json::from_str(&line).unwrap_or(serde_json::Value::Null);
        let cmd = v.get("cmd").and_then(|x| x.as_str()).unwrap_or("");
        let resp = match cmd {
            "open-run" => serde_json::json!({"ok":true,"run_id":1}),
            "ask-human" | "approve" | "step-result" | "advance" => serde_json::json!({"ok":true}),
            _ => serde_json::json!({"ok":false,"error":"unknown cmd"}),
        };
        println!("{}", resp);
    }
}
