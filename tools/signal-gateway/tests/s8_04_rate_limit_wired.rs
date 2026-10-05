//! S8-04 — the rate limiter was a DEAD MODULE, and the proof was its own tests.
//!
//! `RateLimiter::is_allowed` was called from exactly one place: the test module
//! inside `src/ratelimit.rs` itself. `main.rs:27`'s `mod ratelimit;` existed only
//! to make it compile, and `#![allow(dead_code)]` at the top of the file made it
//! compile SILENTLY under a crate that otherwise denies `unwrap_used`,
//! `expect_used` and `panic`. `POST /v2/send` — an outbound primitive that
//! fires real messages over the live Signal identity's websocket — therefore
//! had no request-rate control at all.
//!
//! What the old suite could prove: that a budget fills up. What it could not
//! prove, and what nobody had asked: that the budget DRAINS. `is_allowed` read
//! `Instant::now()` internally with no clock seam, so "the window passed" was
//! not expressible. `admit_at` is that seam.
//!
//! Three layers of proof here, weakest to strongest:
//!   1. behavioural — drive the real limiter, including expiry and eviction;
//!   2. end-to-end  — a real socket on 127.0.0.1, a real `reqwest` client, real
//!      429s with a real `RETRY-AFTER`;
//!   3. structural  — read `src/main.rs` and `src/api/mod.rs` and fail if the
//!      layer is unwired. The defect in this finding IS an unwiring, so the
//!      strongest pin is that deleting the wrap breaks a test.

// Test-only: the crate denies panic vectors in PRODUCTION code (`Cargo.toml`
// `[lints.clippy]`). A test that cannot fail loudly is not a test. Same scoping
// as `src/lib.rs` and `s8_01_bind_coupled_auth.rs`.
#![allow(clippy::expect_used, clippy::panic)]

use axum::Router;
use axum::response::IntoResponse;

use signal_gateway::apply_rate_limit;
use signal_gateway::ratelimit::{
    API_RATE_LIMIT_KEY, API_RATE_LIMIT_MAX_REQUESTS, API_RATE_LIMIT_WINDOW_SECS, RateLimiter,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// ── 1. Behavioural: the decision, at injected instants ───────────────────────

#[test]
fn the_budget_is_a_boundary_not_an_off_by_one() {
    let limiter = RateLimiter::new(3, 60);
    let t0 = Instant::now();
    // Exactly `max_requests` admissions, then the refusal.
    assert!(limiter.admit_at("k", t0), "1st of 3");
    assert!(limiter.admit_at("k", t0), "2nd of 3");
    assert!(
        limiter.admit_at("k", t0),
        "3rd of 3 — the boundary is inclusive"
    );
    assert!(
        !limiter.admit_at("k", t0),
        "the 4th request inside one window must be refused"
    );
}

#[test]
fn the_window_drains_and_admits_again() {
    // The red-proof the old module could not express: with no clock seam, this
    // assertion was literally unrepresentable. 60 seconds of real waiting is
    // not a test; an injected `Instant` is.
    let limiter = RateLimiter::new(2, 60);
    let t0 = Instant::now();
    assert!(limiter.admit_at("k", t0));
    assert!(limiter.admit_at("k", t0));
    assert!(
        !limiter.admit_at("k", t0),
        "budget exhausted inside the window"
    );

    let just_inside = t0 + Duration::from_secs(59);
    assert!(
        !limiter.admit_at("k", just_inside),
        "at 59 s the original entries are still inside the window"
    );

    let just_outside = t0 + Duration::from_secs(61);
    assert!(
        limiter.admit_at("k", just_outside),
        "at 61 s both entries have expired and the budget is whole again"
    );
    assert!(
        limiter.admit_at("k", just_outside),
        "and the reset budget fills the same way"
    );
    assert!(!limiter.admit_at("k", just_outside), "then exhausts again");
}

#[test]
fn a_drained_window_releases_exactly_its_own_entries() {
    // Partial expiry: one of two entries ages out, so exactly one slot frees.
    let limiter = RateLimiter::new(2, 60);
    let t0 = Instant::now();
    assert!(limiter.admit_at("k", t0));
    let t1 = t0 + Duration::from_secs(30);
    assert!(limiter.admit_at("k", t1));
    assert!(!limiter.admit_at("k", t1), "both inside one window");

    // At t0+61 the t0 entry is out; at t0+90 both are out.
    assert!(
        limiter.admit_at("k", t0 + Duration::from_secs(61)),
        "the t0 entry has expired, freeing exactly one slot"
    );
    assert!(!limiter.admit_at("k", t0 + Duration::from_secs(61)));
}

#[test]
fn keys_are_isolated_from_one_another() {
    let limiter = RateLimiter::new(1, 60);
    let t0 = Instant::now();
    assert!(limiter.admit_at("a", t0));
    assert!(!limiter.admit_at("a", t0), "a is exhausted");
    assert!(
        limiter.admit_at("b", t0),
        "b must be unaffected by a's exhaustion"
    );
    assert!(!limiter.admit_at("b", t0));
}

#[test]
fn a_key_that_goes_silent_is_reclaimed_by_the_next_request() {
    let limiter = RateLimiter::new(2, 60);
    let t0 = Instant::now();
    assert_eq!(limiter.tracked_keys(), 0, "nothing is tracked before use");

    assert!(limiter.admit_at("silent", t0));
    assert_eq!(limiter.tracked_keys(), 1);

    // Still inside the window: the entry is live and must be kept.
    assert!(limiter.admit_at("busy", t0 + Duration::from_secs(30)));
    assert_eq!(limiter.tracked_keys(), 2);

    // "silent" has now been quiet for 61 s. A request on ANOTHER key must
    // reclaim it — pruning only on access would never visit it again, and the
    // map would grow with every key that ever appeared. This is the bound the
    // sweep buys: at most one window's worth of distinct keys.
    assert!(limiter.admit_at("busy", t0 + Duration::from_secs(61)));
    assert_eq!(
        limiter.tracked_keys(),
        1,
        "the idle key must be reclaimed by the next request"
    );
}

#[test]
fn a_live_key_is_never_swept_out_from_under_a_request() {
    // The sweep must not reclaim a key that still holds live entries — the
    // budget would silently refill mid-window.
    let limiter = RateLimiter::new(2, 60);
    let t0 = Instant::now();
    assert!(limiter.admit_at("k", t0));
    assert!(limiter.admit_at("k", t0 + Duration::from_secs(30)));
    assert_eq!(limiter.tracked_keys(), 1, "one key, still holding its entries");
    assert!(
        !limiter.admit_at("k", t0 + Duration::from_secs(30)),
        "a swept key would have refilled its budget at 30 s"
    );
}

#[test]
fn the_production_constants_are_the_ones_the_old_code_hardcoded() {
    // Named, not chosen: `create_rate_limiter()` used to read `new(100, 60)`
    // inline. If these move, an operator's actual budget changes silently.
    assert_eq!(API_RATE_LIMIT_MAX_REQUESTS, 100);
    assert_eq!(API_RATE_LIMIT_WINDOW_SECS, 60);
    assert_eq!(API_RATE_LIMIT_KEY, "api");
}

#[test]
fn the_production_limiter_is_built_from_those_constants() {
    let limiter = signal_gateway::create_rate_limiter();
    assert_eq!(limiter.window_secs(), API_RATE_LIMIT_WINDOW_SECS);
    // Spend it, and the next request is over budget — proving the wiring, not
    // just the constant table.
    for _ in 0..API_RATE_LIMIT_MAX_REQUESTS {
        assert!(limiter.is_allowed(API_RATE_LIMIT_KEY));
    }
    assert!(!limiter.is_allowed(API_RATE_LIMIT_KEY));
}

#[test]
fn the_clones_of_a_limiter_share_one_budget() {
    // `apply_rate_limit` clones the limiter into its middleware. If the clone
    // carried its own state, every layer instance would get a fresh budget and
    // the limit would be unenforceable.
    let limiter = RateLimiter::new(2, 60);
    let clone = limiter.clone();
    assert!(limiter.is_allowed("k"));
    assert!(clone.is_allowed("k"));
    assert!(
        !clone.is_allowed("k"),
        "a clone must share the counter, not fork a new budget"
    );
    assert!(!limiter.is_allowed("k"));
}

// ── 2. End-to-end: a real socket, a real HTTP request, a real 429 ───────────
//
// The client here is hand-rolled rather than `reqwest`, which IS already a
// dependency of this package. reqwest 0.13 resolves `rustls-no-provider`, and
// `Client::new()` / `ClientBuilder::build()` then PANIC unless a rustls crypto
// provider is installed — which needs `rustls` as a DIRECT dependency, i.e. a
// new dependency edge. This crate's law for this round is zero new edges, so
// the client is a `GET` written onto a `TcpStream`. Everything under test —
// axum's router, the middleware, the limiter, the socket, the status line, the
// headers — is the real thing.

/// A parsed HTTP/1.1 reply.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

async fn get(addr: SocketAddr, path: &str) -> Reply {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAccept: */*\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write the request");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read the reply");

    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("reply is not a well-formed HTTP/1.1 message: {text:?}"));
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or_else(|| panic!("no numeric status in {status_line:?}"));
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();

    Reply {
        status,
        headers,
        body: body.to_owned(),
    }
}

async fn ping() -> &'static str {
    "pong"
}

/// Serve `limiter`-wrapped `/ping` on a loopback port for the duration of the
/// returned address. Loopback and an ephemeral port: no port collisions, and
/// nothing that could escape the machine.
async fn serve_limited(limiter: RateLimiter) -> SocketAddr {
    let app = apply_rate_limit(Router::new().route("/ping", get_route()), limiter);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener.local_addr().expect("read back the bound address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

/// `get(handler)` needs the handler's `IntoMakeService`; naming it once keeps
/// the route builders below readable.
fn get_route() -> axum::routing::MethodRouter<()> {
    axum::routing::get(ping)
}

#[tokio::test]
async fn past_the_budget_the_real_http_path_returns_429_with_retry_after() {
    let budget = 3;
    let addr = serve_limited(RateLimiter::new(budget, 60)).await;

    for i in 0..budget {
        let reply = get(addr, "/ping").await;
        assert_eq!(
            reply.status, 200,
            "request {i} is inside the budget of {budget} and must be served"
        );
    }

    let refused = get(addr, "/ping").await;
    assert_eq!(
        refused.status, 429,
        "request {budget} is over budget and must be refused over the wire"
    );
    assert_eq!(
        refused.header("retry-after"),
        Some("60"),
        "a 429 must carry RETRY-AFTER so a client knows when to come back"
    );
    assert!(
        refused.body.is_empty(),
        "the refusal body must be empty — it must not echo anything derived \
         from the request: {:?}",
        refused.body
    );
}

#[tokio::test]
async fn the_refusal_persists_across_the_whole_overage_not_just_the_first() {
    let addr = serve_limited(RateLimiter::new(2, 60)).await;
    for i in 0..2 {
        assert_eq!(
            get(addr, "/ping").await.status,
            200,
            "inside the budget ({i})"
        );
    }
    for i in 0..5 {
        assert_eq!(
            get(addr, "/ping").await.status,
            429,
            "overage request {i} must stay refused — the budget must not leak"
        );
    }
}

#[tokio::test]
async fn a_fresh_window_admits_again_through_the_real_http_path() {
    let spent = serve_limited(RateLimiter::new(1, 60)).await;
    assert_eq!(get(spent, "/ping").await.status, 200);
    assert_eq!(get(spent, "/ping").await.status, 429);

    // A limiter with an unspent window stands in for the passage of time here:
    // the point is that the layer's verdict comes from the limiter, not from a
    // counter the middleware kept on its own.
    let fresh = serve_limited(RateLimiter::new(1, 60)).await;
    assert_eq!(
        get(fresh, "/ping").await.status,
        200,
        "a limiter whose window has not been spent must admit"
    );
}

#[tokio::test]
async fn the_limit_sits_outside_auth_and_throttles_unauthenticated_requests() {
    // The layering claim, over a real socket: an inner middleware that refuses
    // everything (standing in for the bearer-token layer) plus the outer rate
    // limit must produce 401s first and 429s after — never the reverse, and
    // never an unbounded run of 401s.
    let inner = Router::new()
        .route("/ping", get_route())
        .layer(axum::middleware::from_fn(
            |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| async move {
                if req.uri().path().is_empty() {
                    next.run(req).await
                } else {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        "missing or invalid bearer token",
                    )
                        .into_response()
                }
            },
        ));
    let app = apply_rate_limit(inner, RateLimiter::new(3, 60));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    for i in 0..3 {
        assert_eq!(
            get(addr, "/ping").await.status,
            401,
            "inside the budget the inner auth layer is what answers ({i})"
        );
    }
    assert_eq!(
        get(addr, "/ping").await.status,
        429,
        "past the budget the OUTER layer answers, throttling unauthenticated \
         floods too — this is the tokenless loopback posture"
    );
}

// ── Anti-vacuity: the harness must be able to see BOTH verdicts ──────────────

#[tokio::test]
async fn a_zero_budget_refuses_everything() {
    let addr = serve_limited(RateLimiter::new(0, 60)).await;
    for i in 0..3 {
        assert_eq!(
            get(addr, "/ping").await.status,
            429,
            "a zero budget admits nothing ({i})"
        );
    }
}

#[tokio::test]
async fn an_unspent_budget_never_refuses() {
    let addr = serve_limited(RateLimiter::new(50, 60)).await;
    for i in 0..25 {
        assert_eq!(
            get(addr, "/ping").await.status,
            200,
            "25 requests against a 50 budget must all be served ({i})"
        );
    }
}

// ── 3. Structural: the finding WAS an unwiring, so pin the unwiring ──────────

fn read(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Does any line START with `needle`, ignoring leading whitespace?
///
/// These pins read files whose doc comments legitimately NAME the things the
/// pins forbid — this very module's header explains the dead `mod ratelimit;`,
/// and the limiter's own docs quote the `#![allow(dead_code)]` it no longer
/// carries. A substring match would therefore be satisfied by prose, which is
/// the opposite of what a structural pin is for.
fn a_line_begins_with(src: &str, needle: &str) -> bool {
    src.lines()
        .any(|line| line.trim_start().starts_with(needle))
}

#[test]
fn the_structural_pins_read_real_source() {
    // Without this, a wrong path would make every assertion below vacuous.
    let main = read("src/main.rs");
    assert!(
        main.contains("fn main()"),
        "src/main.rs did not read back as main"
    );
    assert!(
        read("src/api/mod.rs").contains("create_router_with_auth"),
        "src/api/mod.rs did not read back as the router builder"
    );
    assert!(
        read("src/lib.rs").contains("pub mod ratelimit;"),
        "src/lib.rs must declare the limiter module"
    );
}

#[test]
fn main_builds_the_limiter_and_wraps_the_finished_router() {
    let main = read("src/main.rs");
    assert!(
        main.contains("create_rate_limiter()"),
        "main.rs must build the limiter — a limiter nothing builds is dead again"
    );
    assert!(
        main.contains("apply_rate_limit(app,"),
        "main.rs must wrap the router in the rate limit. Deleting this line is \
         exactly the S8-04 defect: a limiter that exists, compiles, is tested, \
         and is never called."
    );
}

#[test]
fn the_limiter_module_has_exactly_one_definition() {
    // The old shape: `main.rs: mod ratelimit;` made the binary compile its own
    // private copy, which is why integration tests could not reach the real
    // limiter at all.
    let main = read("src/main.rs");
    assert!(
        !a_line_begins_with(&main, "mod ratelimit;"),
        "main.rs must NOT declare `mod ratelimit;` — the limiter belongs to the \
         library target. Two definitions is how a green test suite and a dead \
         module coexisted."
    );
    assert!(
        !a_line_begins_with(&read("src/ratelimit.rs"), "#![allow(dead_code)]"),
        "ratelimit.rs must not carry a blanket dead_code suppression — that \
         blanket is precisely what let a never-called limiter compile silently"
    );
}

#[test]
fn the_rate_limit_wraps_the_auth_arm_and_is_not_hidden_inside_the_builder() {
    // Parenthesis-anchored: the api/mod.rs doc comment discusses the layering by
    // name, but only a CALL has a paren.
    let api = read("src/api/mod.rs");
    assert!(
        !api.contains("apply_rate_limit("),
        "the limiter must NOT be layered inside create_router_with_auth. That \
         function branches on auth_token, so a layer added before the match \
         would sit inside only ONE of the two arms — and the tokenless \
         (loopback) arm would be the one left unlimited."
    );

    let main = read("src/main.rs");
    let arm = main
        .find("let app = if let Some(token)")
        .expect("main.rs must still branch on the auth posture");
    let wrap = main
        .find("apply_rate_limit(app,")
        .expect("main.rs must wrap the router");
    assert!(
        wrap > arm,
        "the limiter must wrap the router AFTER the auth match (wrap at byte \
         {wrap}, auth arm at byte {arm}), so the limit is outermost"
    );
}
