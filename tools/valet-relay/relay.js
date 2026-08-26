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
//        construction.
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

function verifyAlert(secret, id, ts, body, headerSig) {
  if (!headerSig || !headerSig.startsWith('v1,')) return false;
  const expected = sign(secret, id, ts, body);
  const a = Buffer.from(expected);
  const b = Buffer.from(headerSig);
  return a.length === b.length && crypto.timingSafeEqual(a, b);
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

http.createServer((req, res) => {
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
    if (!verifyAlert(CFG.alert_secret, id, ts, body, sig)) {
      console.warn('alert sink: bad signature, refused');
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
}).listen(CFG.listen_port, '127.0.0.1', () => {
  console.log(`valet-relay listening on 127.0.0.1:${CFG.listen_port}`);
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

setInterval(pollOnce, 15000);

// ── self-test mode: `node relay.js --selftest` verifies the two signature ──
// directions without touching any network endpoint.
if (process.argv.includes('--selftest')) {
  const id = 'selftest';
  const ts = '1700000000';
  const body = '{"text":"[case 1] hello"}';
  const sig = sign(CFG.relay_secret, id, ts, body);
  if (!verifyAlert(CFG.relay_secret, id, ts, body, sig)) die('selftest: sign/verify mismatch');
  if (verifyAlert(CFG.relay_secret, id, ts, body + 'x', sig)) die('selftest: tamper not detected');
  if (envelopeToText('valet/due', { what: 'x', run_id: 1, due_at: 1 }) === null) die('selftest: valet/due not mapped');
  if (envelopeToText('workflow', {}) !== null) die('selftest: non-valet kind must not map');
  console.log('valet-relay selftest: ok');
  process.exit(0);
}
