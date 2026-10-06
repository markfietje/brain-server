// S8-02 — the alert sink must ask *when*, not only *who*.
//
// `verifyAlert` proved the envelope was signed by the holder of the alert
// secret. It never asked whether the signature is still *current*, so a
// captured envelope replayed forever, re-firing an operator alert through
// `sendSignal()` until the secret rotated.
//
// Every case below drives the production functions with an INJECTED clock, so
// the tolerance arms are exercised exactly rather than by waiting five minutes.
//
// Zero dependencies: `node --test` against the built-in runner.

// ── fixture ──────────────────────────────────────────────────────────────────
// `loadConfig()` runs at require time and exits 1 without a 0600 file, so the
// fixture directory is built BEFORE the require.

const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const http = require('node:http');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');

const RELAY = path.join(__dirname, 'relay.js');
const ALERT_SECRET = 'alert-secret-for-tests';

// Mirrors `create_rate_limiter`-style two-sided law on the kernel side:
// src/config.rs WEBHOOK_REPLAY_SECS and src/webhook.rs
// WEBHOOK_TS_FUTURE_SKEW_SECS, both 300.
const TOLERANCE_SECS = 300;

function fixtureConfigDir(overrides) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'valet-relay-test-'));
  const cfg = Object.assign(
    {
      signal_send_url: 'http://127.0.0.1:1/v2/send',
      signal_receive_url: 'http://127.0.0.1:1/v2/receive',
      my_number: '+31612345678',
      relay_secret: 'relay-secret-for-tests',
      alert_secret: ALERT_SECRET,
      listen_port: 18799,
      brain_webhook_url: 'http://127.0.0.1:1/webhooks/signal',
    },
    overrides || {},
  );
  const file = path.join(dir, 'signal-relay.json');
  fs.writeFileSync(file, JSON.stringify(cfg));
  // loadConfig() refuses anything looser than 0600.
  fs.chmodSync(file, 0o600);
  return dir;
}

process.env.BRAIN_CONNECTOR_CONFIG_DIR = fixtureConfigDir();

const relay = require(RELAY);
const { sign, verifyAlert, freshTimestamp, envelopeToText } = relay;

// ── helpers ──────────────────────────────────────────────────────────────────

// The kernel's alert sink emits `chrono::Utc::now().to_rfc3339()`
// (src/alert.rs:510): second precision, explicit `+00:00` offset. NOT the `Z`
// form `toISOString()` returns — this mirrors the producer byte-for-byte in
// shape, because a freshness check that only understands one of the two is a
// green suite over a fix that rejects every real envelope.
function kernelRfc3339(ms) {
  return `${new Date(ms).toISOString().slice(0, 19)}+00:00`;
}

const NOW_MS = Date.UTC(2026, 9, 6, 0, 31, 24);
const BODY = JSON.stringify({ kind: 'valet/due', payload: { what: 'x', run_id: 1, due_at: 1 } });

// A correctly-signed envelope: the MAC is always valid, so every refusal below
// is attributable to the freshness gate alone.
function signedEnvelope(ts) {
  const id = 'msg_under_test';
  return { id, ts, body: BODY, sig: sign(ALERT_SECRET, id, ts, BODY) };
}

function accepts(ts, nowMs) {
  const e = signedEnvelope(ts);
  return verifyAlert(ALERT_SECRET, e.id, e.ts, e.body, e.sig, nowMs);
}

// ── the format the real producer actually emits ──────────────────────────────

test('a fresh RFC3339 timestamp is accepted', () => {
  // The kernel sends RFC3339, NOT the epoch-seconds the Standard Webhooks spec
  // defines. This is the case a fresh-epoch-only suite would silently fail.
  assert.equal(accepts(kernelRfc3339(NOW_MS), NOW_MS), true);
});

test('a fresh RFC3339 timestamp is accepted one second either side of now', () => {
  const ts = kernelRfc3339(NOW_MS);
  assert.equal(accepts(ts, NOW_MS + 1000), true);
  assert.equal(accepts(ts, NOW_MS - 1000), true);
});

// ── the spec-conformant producer ─────────────────────────────────────────────

test('a fresh epoch-seconds timestamp is accepted', () => {
  assert.equal(accepts(String(NOW_MS / 1000), NOW_MS), true);
});

// ── the S8-02 replay, refused on both formats ────────────────────────────────

test('RFC3339 exactly one second past the tolerance is refused', () => {
  const ts = kernelRfc3339(NOW_MS - (TOLERANCE_SECS + 1) * 1000);
  assert.equal(accepts(ts, NOW_MS), false, 'a stale RFC3339 envelope must be refused');
});

test('epoch exactly one second past the tolerance is refused', () => {
  const ts = String((NOW_MS - (TOLERANCE_SECS + 1) * 1000) / 1000);
  assert.equal(accepts(ts, NOW_MS), false, 'a stale epoch envelope must be refused');
});

test('the tolerance boundary itself is inclusive', () => {
  // 300 s is inside the law, 301 s is outside. A test that only probes "very
  // old" cannot tell a 300 s window from a 60 s one.
  const freshest = kernelRfc3339(NOW_MS - TOLERANCE_SECS * 1000);
  assert.equal(accepts(freshest, NOW_MS), true, 'exactly 300 s old is inside the window');
});

// ── the future arm, mirroring enqueue_ts's two-sided law ─────────────────────

test('a future-dated timestamp past the skew is refused', () => {
  const rfc = kernelRfc3339(NOW_MS + (TOLERANCE_SECS + 1) * 1000);
  assert.equal(accepts(rfc, NOW_MS), false, 'future skew must be bounded too');
  const epoch = String((NOW_MS + (TOLERANCE_SECS + 1) * 1000) / 1000);
  assert.equal(accepts(epoch, NOW_MS), false);
});

test('future skew inside the window is accepted', () => {
  const ts = kernelRfc3339(NOW_MS + TOLERANCE_SECS * 1000);
  assert.equal(accepts(ts, NOW_MS), true);
});

// ── fail-closed on anything unparsable ───────────────────────────────────────

test('an unparsable timestamp is refused even with a valid MAC', () => {
  for (const ts of ['', 'not-a-timestamp', 'yesterday', '17e9', '0x5f5e100']) {
    assert.equal(accepts(ts, NOW_MS), false, `ts=${JSON.stringify(ts)} must fail closed`);
  }
});

test('a missing timestamp header is refused', () => {
  const id = 'msg_under_test';
  const sig = sign(ALERT_SECRET, id, '', BODY);
  assert.equal(verifyAlert(ALERT_SECRET, id, '', BODY, sig, NOW_MS), false);
});

// ── the pre-existing guarantees, pinned ──────────────────────────────────────

test('a tampered body is still refused', () => {
  const e = signedEnvelope(kernelRfc3339(NOW_MS));
  assert.equal(verifyAlert(ALERT_SECRET, e.id, e.ts, `${e.body}x`, e.sig, NOW_MS), false);
});

test('a signature without the v1, prefix is still refused', () => {
  const e = signedEnvelope(kernelRfc3339(NOW_MS));
  assert.equal(verifyAlert(ALERT_SECRET, e.id, e.ts, e.body, e.sig.slice(3), NOW_MS), false);
  assert.equal(verifyAlert(ALERT_SECRET, e.id, e.ts, e.body, '', NOW_MS), false);
});

test('a body signed with the wrong secret is still refused', () => {
  const ts = kernelRfc3339(NOW_MS);
  const sig = sign('a-different-secret', 'msg_under_test', ts, BODY);
  assert.equal(verifyAlert('a-different-secret', 'msg_under_test', ts, BODY, sig, NOW_MS), true);
  assert.equal(verifyAlert(ALERT_SECRET, 'msg_under_test', ts, BODY, sig, NOW_MS), false);
});

// ── no id-dedup: the producer retries with the SAME id and the SAME ts ───────

test('the same id and ts is admitted twice — retries must not be eaten', () => {
  // src/alert.rs:508-535 sets `ts` ONCE and retries up to 3 times with the same
  // delivery_id. An id-dedup here would silently drop a legitimate retry whose
  // response was lost after the forward. The fix is freshness, NOT dedup, and
  // this test is what holds that decision in place.
  const e = signedEnvelope(kernelRfc3339(NOW_MS));
  const first = verifyAlert(ALERT_SECRET, e.id, e.ts, e.body, e.sig, NOW_MS);
  const second = verifyAlert(ALERT_SECRET, e.id, e.ts, e.body, e.sig, NOW_MS);
  assert.equal(first, true);
  assert.equal(second, true);
});

// ── the seam defaults to the real clock (no test-only clock leaks out) ───────

test('with no injected clock a genuinely stale envelope is still refused', () => {
  const stale = String(Math.floor((Date.now() - (TOLERANCE_SECS + 5) * 1000) / 1000));
  const e = signedEnvelope(stale);
  assert.equal(verifyAlert(ALERT_SECRET, e.id, e.ts, e.body, e.sig), false);
});

// ── the pure decision, named ─────────────────────────────────────────────────

test('freshTimestamp is the gate on its own, and takes milliseconds', () => {
  assert.equal(freshTimestamp(kernelRfc3339(NOW_MS), NOW_MS), true);
  assert.equal(freshTimestamp(String(NOW_MS / 1000), NOW_MS), true);
  assert.equal(freshTimestamp(kernelRfc3339(NOW_MS - 301_000), NOW_MS), false);
  assert.equal(freshTimestamp(kernelRfc3339(NOW_MS + 301_000), NOW_MS), false);
  assert.equal(freshTimestamp(undefined, NOW_MS), false);
  assert.equal(freshTimestamp(null, NOW_MS), false);
  assert.equal(freshTimestamp(1_700_000_000, NOW_MS), false, 'a non-string header is refused');
});

// ── the kinds rule is untouched by this round ────────────────────────────────

test('the forwarded-kind rule is unchanged', () => {
  assert.notEqual(envelopeToText('valet/due', { what: 'x', run_id: 1, due_at: 1 }), null);
  assert.equal(envelopeToText('workflow', {}), null);
});

// ── the inbound dedup id keys on the ENVELOPE, not the clock ─────────────────

test('a retained envelope keeps its dedup id across re-polls', () => {
  const env = { envelope: { timestamp: 1727712000000, source: '+15550001111',
    dataMessage: { message: 'what is due' } } };
  // The platform timestamp rides the id — NOT the wall clock. A time-of-
  // forward id lets the same envelope re-post once per second, which is
  // exactly the replay the client-side cap exists to catch.
  const id = relay.inboundDedupId(env, 'what is due', '+15550001111');
  assert.match(id, /^signal-1727712000000-[0-9a-f]{12}$/,
    'the id must derive from the envelope timestamp');
  assert.equal(id, relay.inboundDedupId(env, 'what is due', '+15550001111'),
    'same envelope, same id — across any two poll instants');
  // Distinct envelopes stay distinct (anti-vacuity: not a constant id).
  const other = relay.inboundDedupId(env, 'different text', '+15550001111');
  assert.notEqual(id, other, 'the text still discriminates');
});

// ── end-to-end: a real listener, a real child process, a real Signal sink ────

function listen(server) {
  return new Promise((resolve, reject) => {
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => resolve(server.address().port));
  });
}

function freePort() {
  return new Promise((resolve, reject) => {
    const probe = net.createServer();
    probe.on('error', reject);
    probe.listen(0, '127.0.0.1', () => {
      const port = probe.address().port;
      probe.close(() => resolve(port));
    });
  });
}

function waitForStdout(child, needle) {
  return new Promise((resolve, reject) => {
    let seen = '';
    const timer = setTimeout(() => reject(new Error(`relay never printed "${needle}": ${seen}`)), 10_000);
    const check = (chunk) => {
      seen += chunk;
      if (seen.includes(needle)) {
        clearTimeout(timer);
        resolve();
      }
    };
    child.stdout.on('data', check);
    child.stderr.on('data', check);
    child.on('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`relay exited early with ${code}: ${seen}`));
    });
  });
}

function postAlert(port, ts, id, sig) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port,
        path: '/alert',
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          'content-length': Buffer.byteLength(BODY),
          'webhook-id': id,
          'webhook-timestamp': ts,
          'webhook-signature': sig,
        },
      },
      (res) => {
        res.resume();
        res.on('end', () => resolve(res.statusCode));
      },
    );
    req.on('error', reject);
    req.end(BODY);
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

test('the live listener forwards a fresh envelope and refuses a replayed one', async () => {
  const forwards = [];
  const sink = http.createServer((req, res) => {
    if (req.method === 'POST' && req.url === '/v2/send') {
      forwards.push(req.url);
      res.writeHead(200, { 'content-type': 'application/json' }).end('{}');
      return;
    }
    // The inbound poll: an empty envelope list is a quiet Signal account.
    res.writeHead(200, { 'content-type': 'application/json' }).end('[]');
  });
  const sinkPort = await listen(sink);
  const relayPort = await freePort();
  const cfgDir = fixtureConfigDir({
    signal_send_url: `http://127.0.0.1:${sinkPort}/v2/send`,
    signal_receive_url: `http://127.0.0.1:${sinkPort}/v2/receive`,
    listen_port: relayPort,
  });

  const child = spawn(process.execPath, [RELAY], {
    env: Object.assign({}, process.env, { BRAIN_CONNECTOR_CONFIG_DIR: cfgDir }),
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  try {
    await waitForStdout(child, 'listening');

    // Control first: a fresh envelope MUST reach Signal. Without this the 401
    // below could be a dead child rather than the freshness gate.
    const nowTs = String(Math.floor(Date.now() / 1000));
    const freshSig = sign(ALERT_SECRET, 'msg_fresh', nowTs, BODY);
    assert.equal(await postAlert(relayPort, nowTs, 'msg_fresh', freshSig), 200);
    assert.deepEqual(forwards, ['/v2/send'], 'a fresh alert must forward exactly once');

    // The replay: a valid MAC over a timestamp from outside the window.
    const staleTs = String(Math.floor(Date.now() / 1000) - (TOLERANCE_SECS + 1));
    const staleSig = sign(ALERT_SECRET, 'msg_fresh', staleTs, BODY);
    assert.equal(
      await postAlert(relayPort, staleTs, 'msg_fresh', staleSig),
      401,
      'a replayed envelope must be refused at the sink',
    );

    await sleep(250);
    assert.deepEqual(forwards, ['/v2/send'], 'a refused envelope must never reach Signal');
  } finally {
    child.kill('SIGKILL');
    sink.close();
  }
});