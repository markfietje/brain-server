# valet-relay

The Signal bridge edge for brain-server's Valet (v1.28.42). A zero-dependency
Node process. See the header comment of `relay.js` for the full contract.

## What it is allowed to do

- OUT: listen on `127.0.0.1:$listen_port/alert` as the server's
  `BRAIN_ALERT_WEBHOOK_URL` sink; verify the Standard Webhooks signature;
  forward ONLY `valet/due` (and later `valet/brief`) alert envelopes to your
  number via signal-cli-rest-api. Metadata-only by construction.
- IN: poll signal-cli's receive endpoint; for messages from YOUR number only,
  sign them Standard-Webhooks style and POST to brain-server's
  `/webhooks/signal` (the server verifies, replay-caps, and injection-screens
  every byte before any state change).

## What it is forbidden from doing (pinned by `relay_holds_no_brain_credentials`)

- No brain token, no `auth-token` path, no `Authorization` header, no
  `brain.db` access. It reaches exactly two endpoints: the alert sink it
  hosts and `/webhooks/signal` it calls.

## Config

`$BRAIN_CONNECTOR_CONFIG_DIR/signal-relay.json` (chmod 600):

```json
{
  "signal_send_url": "http://127.0.0.1:8080/v2/send",
  "signal_receive_url": "http://127.0.0.1:8080/v2/receive",
  "my_number": "+31612345678",
  "relay_secret": "<hex secret mirrored in BRAIN_SIGNAL_WEBHOOK_SECRET_FILE on the server>",
  "alert_secret": "<hex secret mirrored in BRAIN_ALERT_WEBHOOK_SECRET on the server>",
  "listen_port": 8790,
  "brain_webhook_url": "http://127.0.0.1:8765/webhooks/signal"
}
```

## Run

```sh
node tools/valet-relay/relay.js --selftest   # signature round-trip, no network
node tools/valet-relay/relay.js              # the relay (launchd/cron keeps it alive)
```

If the relay dies, reminders queue in the server's outbox — the morning
still exists.
