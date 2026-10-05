#!/usr/bin/env node
// valet-relay — the Signal bridge edge for brain-server's Valet (v1.28.42).
//
// A small, zero-dependency Node process mirroring the plugin's ladder
// conventions. It holds ONLY its own 0600 secrets (signal-cli endpoint, your
// number, the relay HMAC secret) and can reach exactly TWO endpoints:
//
//   OUT: brain-server's alert webhook sink (this process LISTENS; the server
//        POSTs signed alert envelopes here via BRAIN_ALERT_WEBHOOK_URL).
//        Only `valet/due` (and later `valet/brief`) kinds are forwarded to
//        Signal — nothing else, ever. The alert bus carries metadata-only
//        envelopes by construction, so what arrives here is sanitized by
//        construction. Each envelope must be signed AND recent (±300 s), so a
//        captured one cannot replay forever.
//
//   IN:  Signal messages from you are signed Standard-Webhooks style and
//        POSTed to brain-server's /webhooks/signal (the server verifies the
//        HMAC, replay-caps, and injection-screens every byte).
//
// It NEVER holds a brain token, NEVER touches the database, and NEVER sends
// anything the human did not ask for: outbound is exactly alert-envelope
// forwards; inbound is exactly your commands. If this process dies,
// reminders queue in the server's outbox — the morning still exists.
//
// Config: $BRAIN_CONNECTOR_CONFIG_DIR/signal-relay.json (0600), fields:
//   {
//     "signal_send_url": "http://127.0.0.1:8080/v2/send",  // signal-cli-rest-api
//     "signal_receive_url": "http://127.0.0.1:8080/v2/receive",
//     "my_number": "+31612345678",
//     "relay_secret": "<hex used to sign INBOUND webhooks to the server>",
//     "alert_secret": "<hex the server uses to sign OUTBOUND alert envelopes>",
//     "listen_port": 8790,
//     "brain_webhook_url": "http://127.0.0.1:8765/webhooks/signal"
//   }

'use strict';

const http = require('http');
const crypto = require('crypto');
const fs = require('fs');

function die(msg) {
  console.error(`valet-relay: ${msg}`);
  process.exit(1);
}

function loadConfig() {
  const dir = process.env.BRAIN_CONNECTOR_CONFIG_DIR ||
    `${process.env.HOME}/.config/brain-server/connectors`;
  const path = `${dir}/signal-relay.json`;
  const stat = fs.statSync(path);
  if ((stat.mode & 0o077) !== 0) die(`config ${path} must be 0600`);
  const cfg = JSON.parse(fs.readFileSync(path, 'utf8'));
  for (const k of ['signal_send_url', 'signal_receive_url', 'my_number',
    'relay_secret', 'alert_secret', 'listen_port', 'brain_webhook_url']) {
    if (!cfg[k]) die(`config missing ${k}`);
  }
  return cfg;
}

const CFG = loadConfig();

// ── Standard Webhooks signatures (same scheme the server verifies/signs) ──

function sign(secret, id, ts, body) {
  const mac = crypto.createHmac('sha256', secret)
    .update(`${id}.${ts}.${body}`)
    .digest('base64');
  return `v1,${mac}`;
}

// How far `webhook-timestamp` may sit from "now", in EITHER direction, before
// the envelope is treated as a replay (too old) or a forgery (future skew).
// Three authorities agree on 300 — this is a mirrored law, not a chosen knob:
//   * the Standard Webhooks spec's reference implementation (TOLERANCE_IN_SECONDS
//     = 5 * 60; rejects when `now - ts > 300` or `ts > now + 300`);
//   * the kernel's own inbound replay window (src/config.rs WEBHOOK_REPLAY_SECS
//     = 300, enforced in `enqueue_ts`);
//   * the kernel's own future-skew allowance (src/webhook.rs
//     WEBHOOK_TS_FUTURE_SKEW_SECS = 300 — the other arm of that same `if`).
// Deliberately NOT an env var: this repo's env-truth gate treats an undocumented
// knob as a finding, and an operator-facing surface here buys nothing.
const FRESHNESS_TOLERANCE_SECS = 300;

// Is `tsHeader` recent enough to accept? `nowMs` is milliseconds, matching
// `Date.now()`.
//
// TWO formats must parse, because the only real producer sends the second one:
// the kernel's alert sink signs with `chrono::Utc::now().to_rfc3339()`
// (src/alert.rs:510) — RFC3339 — while the Standard Webhooks spec DEFINES this
// header as epoch seconds. An epoch-only parser would `NaN` on every genuine
// kernel envelope: a green suite over a fix that rejects all legitimate traffic.
// So: all-digits → epoch seconds; anything else → RFC3339 via `Date.parse`.
//
// Fail-closed throughout. A missing, non-string, or unparsable header is
// `false` — never a throw, and never an admission.
function freshTimestamp(tsHeader, nowMs) {
  if (typeof tsHeader !== 'string' || tsHeader.length === 0) return false;
  const t = /^\d+$/.test(tsHeader) ? Number(tsHeader) * 1000 : Date.parse(tsHeader);
  if (!Number.isFinite(t)) return false;
  const driftSecs = Math.floor(Math.abs(nowMs - t) / 1000);
  return driftSecs <= FRESHNESS_TOLERANCE_SECS;
}

// Two INDEPENDENT gates, both of which must pass: the MAC proves WHO signed
// this exact `${id}.${ts}.${body}`, and freshness proves it was signed *now*.
// The MAC is checked first — that is the cheap, decisive one — and the order is
// immaterial to the outcome, since neither short-circuits into admitting.
//
// `nowMs` exists so tests can inject a clock instead of waiting five minutes
// (the seam-and-delegate shape the Rust side uses); it defaults to the real one,
// so no caller can accidentally verify freshness against a frozen clock.
function verifyAlert(secret, id, ts, body, headerSig, nowMs = Date.now()) {
  if (!headerSig || !headerSig.startsWith('v1,')) return false;
  const expected = sign(secret, id, ts, body);
  const a = Buffer.from(expected);
  const b = Buffer.from(headerSig);
  if (a.length !== b.length || !crypto.timingSafeEqual(a, b)) return false;
  return freshTimestamp(ts, nowMs);
}

// ── OUT: the alert sink listener ────────────────────────────────────────────
// Forwards ONLY alert envelopes of kind valet/due (metadata-only by
// construction). Anything else on this port is refused and logged — the
// relay is not a general-purpose forwarder.

function postJson(url, headers, body) {
  return new Promise((resolve) => {
    const u = new URL(url);
    const req = http.request({
      hostname: u.hostname, port: u.port, path: u.pathname + u.search,
      method: 'POST', headers: { ...headers, 'content-type': 'application/json' },
      timeout: 15000,
    }, (res) => {
      let data = '';
      res.on('data', (c) => { data += c; });
      res.on('end', () => resolve({ status: res.statusCode, body: data }));
    });
    req.on('error', (e) => resolve({ status: 0, body: String(e) }));
    req.end(body);
  });
}

function sendSignal(text) {
  // signal-cli-rest-api v2/send: number, message; recipients = [my_number].
  const payload = JSON.stringify({
    number: CFG.my_number,
    recipients: [CFG.my_number],
    message: text,
  });
  return postJson(CFG.signal_send_url, {}, payload);
}

function envelopeToText(kind, payload) {
  if (kind === 'valet/due') {
    // The drained alert envelope nests the original outbox payload as a JSON
    // STRING under payload_json — unwrap it, fall back to the envelope.
    let inner = payload;
    if (typeof payload.payload_json === 'string') {
      try { inner = JSON.parse(payload.payload_json); } catch { /* keep envelope */ }
    }
    const what = typeof inner.what === 'string' ? inner.what : 'reminder';
    const due = typeof inner.due_at === 'number'
      ? new Date(inner.due_at * 1000).toISOString() : '?';
    const run = typeof inner.run_id === 'number' ? inner.run_id : payload.run_id;
    return `[valet] due: ${what} (run ${run}, due ${due})`;
  }
  return null; // unknown kind: never forwarded
}

// Built at module scope but NOT bound: constructing an http.Server opens no
// socket and holds no handle, so `require`ing this file from a test starts
// nothing. The bind lives under the `require.main` guard below.
const alertServer = http.createServer((req, res) => {
  if (req.method !== 'POST' || req.url !== '/alert') {
    res.writeHead(404).end();
    return;
  }
  let body = '';
  req.on('data', (c) => { body += c; });
  req.on('end', async () => {
    const id = req.headers['webhook-id'] || '';
    const ts = req.headers['webhook-timestamp'] || '';
    const sig = req.headers['webhook-signature'] || '';
    // Two independent gates: a valid MAC (who signed it) AND a recent ts (when).
    // Both must hold, so one captured, still-valid envelope is refused here.
    if (!verifyAlert(CFG.alert_secret, id, ts, body, sig)) {
      console.warn('alert sink: bad signature or stale timestamp, refused');
      res.writeHead(401).end();
      return;
    }
    let event;
    try { event = JSON.parse(body); } catch { res.writeHead(400).end(); return; }
    // The ONE outbound rule: only alert envelopes, only the valet kinds.
    const text = envelopeToText(event.kind, event.payload || {});
    if (text === null) {
      console.log(`alert sink: ignored kind ${event.kind} (not forwarded)`);
      res.writeHead(200).end();
      return;
    }
    const r = await sendSignal(text);
    console.log(`signal send: http ${r.status}`);
    res.writeHead(r.status >= 200 && r.status < 300 ? 200 : 502).end();
  });
});

// ── IN: poll Signal, sign + POST to /webhooks/signal ────────────────────────

const seenEnvelopes = new Set(); // bounded below; replay protection client-side

async function pollOnce() {
  let r;
  await new Promise((resolve) => {
    const u = new URL(CFG.signal_receive_url);
    const req = http.request({
      hostname: u.hostname, port: u.port, path: u.pathname, method: 'GET',
      timeout: 15000,
    }, (res) => {
      let data = '';
      res.on('data', (c) => { data += c; });
      res.on('end', () => resolve({ status: res.statusCode, body: data }));
    });
    req.on('error', () => resolve({ status: 0, body: '' }));
    req.end();
  }).then((x) => { r = x; });
  if (!r || r.status !== 200) return;
  let envelopes;
  try { envelopes = JSON.parse(r.body); } catch { return; }
  if (!Array.isArray(envelopes)) return;
  for (const env of envelopes) {
    const text = env && env.envelope && env.envelope.dataMessage &&
      env.envelope.dataMessage.message;
    const from = env && env.envelope && env.envelope.source;
    if (typeof text !== 'string') continue;
    if (from !== CFG.my_number) continue; // only MY commands steer the brain
    const ts = String(Math.floor(Date.now() / 1000));
    const id = `signal-${ts}-${crypto.createHash('sha1').update(text + from).digest('hex').slice(0, 12)}`;
    if (seenEnvelopes.has(id)) continue;
    seenEnvelopes.add(id);
    if (seenEnvelopes.size > 500) {
      // bound the set: drop the oldest half (Map would be nicer; Set iterates
      // insertion order, so this is FIFO).
      let i = 0;
      for (const k of seenEnvelopes) {
        if (i++ < 250) seenEnvelopes.delete(k); else break;
      }
    }
    const body = JSON.stringify({ text, from });
    const sig = sign(CFG.relay_secret, id, ts, body);
    const resp = await postJson(CFG.brain_webhook_url, {
      'webhook-id': id,
      'webhook-timestamp': ts,
      'webhook-signature': sig,
    }, body);
    console.log(`inbound forwarded (${id}): http ${resp.status}`);
  }
}

// ── entry point ──────────────────────────────────────────────────────────────
// The three side effects above — bind the sink, start the poll timer, run the
// self-test — happen only when this file IS the process. `require`-ing it (as
// `relay.test.js` does) yields the pure decision functions and nothing else.
// No behaviour changes when it is run as a process.

module.exports = { sign, verifyAlert, freshTimestamp, envelopeToText, FRESHNESS_TOLERANCE_SECS };

if (require.main === module) {
  // ── self-test mode: `node relay.js --selftest` verifies the two signature
  // directions and the freshness gate without touching any network endpoint.
  if (process.argv.includes('--selftest')) {
    const id = 'selftest';
    const body = '{"text":"[case 1] hello"}';
    const nowMs = Date.now();
    const freshTs = String(Math.floor(nowMs / 1000));
    const staleTs = String(Math.floor(nowMs / 1000) - (FRESHNESS_TOLERANCE_SECS + 60));
    const sig = sign(CFG.relay_secret, id, freshTs, body);
    if (!verifyAlert(CFG.relay_secret, id, freshTs, body, sig, nowMs)) die('selftest: sign/verify mismatch');
    if (verifyAlert(CFG.relay_secret, id, freshTs, body + 'x', sig, nowMs)) die('selftest: tamper not detected');
    // Freshness: a fresh timestamp is admitted, the same envelope replayed from
    // outside the window is refused even though its MAC is still valid.
    const staleSig = sign(CFG.relay_secret, id, staleTs, body);
    if (verifyAlert(CFG.relay_secret, id, staleTs, body, staleSig, nowMs)) die('selftest: stale timestamp accepted');
    if (!freshTimestamp(new Date(nowMs).toISOString(), nowMs)) die('selftest: RFC3339 timestamp rejected');
    if (freshTimestamp('not-a-timestamp', nowMs)) die('selftest: unparsable timestamp accepted');
    if (envelopeToText('valet/due', { what: 'x', run_id: 1, due_at: 1 }) === null) die('selftest: valet/due not mapped');
    if (envelopeToText('workflow', {}) !== null) die('selftest: non-valet kind must not map');
    console.log('valet-relay selftest: ok');
    process.exit(0);
  }

  alertServer.listen(CFG.listen_port, '127.0.0.1', () => {
    console.log(`valet-relay listening on 127.0.0.1:${CFG.listen_port}`);
  });

  setInterval(pollOnce, 15000);
}
