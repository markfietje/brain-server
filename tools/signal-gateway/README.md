# signal-gateway

A lightweight Signal daemon edge for **brain-server's Switchboard** channel
line. Rust-native via [presage](https://github.com/whisperfish/presage) — no
signal-cli/JVM dependency.

**Role (the governed-edge law):** this process holds ONLY its own credentials
(the presage Signal store + its 0600 config). It NEVER holds a brain-server
token and NEVER touches brain storage — the kernel stays channel-free by
construction, pinned by `bridge_holds_no_brain_credentials` upstream.

## Layout

- `src/signal/` — presage worker: link/load account, send/receive, reactions,
  typing; recipient cache (phone → UUID), rate-limited command loop.
- `src/api/` — local HTTP surface: health, account info, send v2,
  JSON-RPC (`sendMessage`, `sendReaction`, …), SSE message stream.
- `src/cache.rs` / `src/validation.rs` — the safety floor: bounded channels,
  input validation, semaphore throttling.
- `src/ratelimit.rs` — the HTTP **request-rate** limit. Lives in the library
  target (`src/lib.rs`) so the binary and the integration tests share one
  definition; `apply_rate_limit` puts it on the request path.

## Request-rate posture

Every request is admitted against **one global budget: 100 requests per 60 s**
(`API_RATE_LIMIT_MAX_REQUESTS` / `API_RATE_LIMIT_WINDOW_SECS`, applied to
`API_RATE_LIMIT_KEY`). Over budget the response is a bare `429` with
`RETRY-AFTER: 60` and an empty body — no request-derived data in the reply or
the log line.

- **Layered OUTSIDE auth.** `main.rs` wraps the finished router, so the limit is
  the outermost thing a request meets and a flood of *unauthenticated* requests
  is bounded in the tokenless loopback posture too.
- **Global, not per-IP, on purpose.** The server is `axum::serve(listener,
  app)` with no `ConnectInfo`, and under the loopback posture every client is
  `127.0.0.1` anyway — per-IP keying would read as control while being an
  illusion. Behind a proxy it collapses to one address regardless. The limiter
  itself stays generic over its key, so per-IP is a call-site change, not a
  rewrite.
- **Distinct from the send semaphore.** `max_sends_per_second` in
  `config.yaml` is a *concurrency* cap (5 in-flight sends), not a rate limit.
  The two bounds answer different questions and both are live.
- **Not operator-tunable.** 100/60 are named constants in the library, not
  config keys — a knob here would need env-truth + docs + example-yaml churn
  and no deployment evidence demands it.

Stated limits: a burst of 100 still reaches Signal, and the SSE long-poll on
`/api/v1/events` draws from the same budget as `/v2/send`.

## Privacy posture (hidden & anonymous)

Signal accounts are born on a phone number — but the number never has to be
VISIBLE:

1. **On the primary phone app (one-time):** create a Signal username, then
turn OFF *“Allow people who have my number to find me”*. Optionally reset the
username link. From then on contacts see only the username.
2. **In this gateway:** set `signal.display_name` in config.yaml to that
username — every API response, log line and broadcast payload then carries
the label instead of any number form; unset falls back to masked digits
(`+63…67`). Recipient addressing accepts usernames directly: the worker
resolves them server-side via presage's `lookup_username` and caches the ACI.
3. **In brain-server:** envelopes carry only conversation UUIDs; identity is
HASHED (`subject_hash`) before anything rests in the thread map or registry.

Honest ceiling: Signal's servers still know the account's number (protocol
truth) and fully self-registering a number-less account isn't supported by
upstream — anonymity here is from CONTACTS AND OBSERVERS, not from Signal Corp.

## Use

```sh
signal-gateway link   --config config.yaml --device-name signal-gateway
signal-gateway serve  --config config.yaml
```

Config example lives in `config.example.yaml` (keep it 0600 when real).

Version tracks the libsignal stack in `Cargo.lock` (currently 0.99.0);
`#![forbid(unsafe_code)]` is compile-enforced.
