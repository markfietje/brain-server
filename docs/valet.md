# Valet (personal reminders)

Valet is a small reminder keeper with a consent gate. A run says what and
when; the crank fires what is due; the brief reads the morning back. There
is no daemon and no scheduler inside the server. Cron (or anything else
that can POST) invokes the crank.

## HTTP

All three routes need the `workflow` role.

- `POST /workflow/valet/due` (Write) fires every due `valet/%` run, each in
  its own audited transaction with an idempotency key, and re-arms repeats
  by compare-and-swap. Runs without consent are suppressed and counted,
  not fired. Body: `{now?}`.
- `GET /workflow/valet/brief` (Read) returns due and overdue runs, pending
  drafts with advisory lint scores, evening notes, and whether Signal
  consent is in force. Read-only and sanitized.
- `PUT /workflow/valet/consent` (Write) records consent for exactly one
  subject (`owner`) on exactly one channel (`signal`), hashed at rest.

## CLI

- `brain valet add "what" --at <time> [--repeat none|daily|weekly]
  [--domain D]` queues a reminder (default domain `personal`; the text is
  injection-screened client-side, capped at 500 characters).
- `brain valet due [--now <unix>]` runs the crank, prints
  fired / suppressed-no-consent / already-fired counts.
- `brain valet brief` prints the DUE / DRAFT / NOTE sections plus consent.
- `brain valet consent grant|revoke` flips the gate.

## Delivery edge

The Signal relay is a separate config-off process holding no brain token.
It only touches its alert sink and the Signal webhook. Secrets live in
0600 files on both sides; see the relay README under `tools/valet-relay`.
