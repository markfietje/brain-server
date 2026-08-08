---
name: Bug report
about: Report a defect in brain-server or the brain-client
title: "[bug] "
labels: bug
assignees: ""
---

## Description

A clear and concise description of the bug.

## Environment

- brain-server version: (e.g. `v1.16.7`)
- Client: web / desktop / iOS / Android (which panels if relevant)
- Platform / OS: (e.g. macOS arm64, Linux x86_64, Jetson)
- Auth mode: loopback (opaque token) / JWT / none
- Rust toolchain (if building locally): (e.g. stable 1.97)

## Reproduction steps

1. …
2. …
3. …

## Expected behavior

What you expected to happen.

## Actual behavior

What actually happened. Include the exact error message or HTTP status code if
relevant (e.g. `401`, `503`, `SQLITE_BUSY`).

## Logs / diagnostics

Run `brain doctor` and `brain status` against the live server and paste the
relevant output. Include the launchd logs if applicable:
`~/Library/Logs/brain-server.{log,err.log}`.

## Notes

- **Security issue?** Do **not** file this publicly. Use the GitHub "Report a
  vulnerability" tab or email security@openclaw.dev (see `SECURITY.md`).
- Have you checked `CHANGELOG.md` to see if this is already fixed in a newer
  release?
