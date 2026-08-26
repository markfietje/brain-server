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
- `src/cache.rs` / `src/validation.rs` / `src/ratelimit.rs` — the safety
  floor: bounded channels, input validation, semaphore throttling.

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
