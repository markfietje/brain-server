// Typed client example: issue a v0.9.5 structured `QueryDoc` against a running
// brain-server and print the hits. Uses the same dependency-free HTTP client the
// `brain` CLI ships, so the example stays in lockstep with the real wire contract.
//
// Usage: cargo run --example client_example -- "your query" [--phrase "exact phrase"]

#[path = "../src/bin_common/http.rs"]
mod http;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let q = match args.first() {
        Some(q) => q.clone(),
        None => {
            eprintln!("usage: client_example <query> [--phrase P]");
            std::process::exit(2);
        }
    };
    let phrases: Vec<String> = args
        .iter()
        .skip(1)
        .filter(|a| *a == "--phrase")
        .filter_map(|_| args.get(args.iter().position(|x| x == "--phrase").unwrap()))
        .cloned()
        .collect();

    let mut body = serde_json::json!({ "query": q, "limit": 5 });
    if !phrases.is_empty() {
        body["lex"] = serde_json::json!({ "phrases": phrases });
    }

    let resp = http::post(
        "http://127.0.0.1:8765",
        "/recall",
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )
    .expect("request failed");
    if resp.status != 200 {
        eprintln!("server error {}: {}", resp.status, resp.body);
        std::process::exit(1);
    }
    let v: serde_json::Value = serde_json::from_str(&resp.body).expect("non-JSON response");
    let hits = v
        .get("hits")
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();
    println!("{} hit(s):", hits.len());
    for h in &hits {
        let id = h.get("id").and_then(|x| x.as_i64()).unwrap_or(-1);
        let score = h.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let title = h
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or("(untitled)");
        println!("  [{:.4}] id={} title={}", score, id, title);
    }
}

fn auth_token() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let p = std::path::Path::new(&home).join(".config/brain-server/auth-token");
    std::fs::read_to_string(&p)
        .ok()
        .map(|s| s.trim().to_string())
}
