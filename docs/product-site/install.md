# Install

Brain Server is one self-hosted runtime (the server binary, plus the `brain`
CLI, `mcp`, and `bench` from the same workspace build). It runs on a
4 GB ARM device up to a beefy server — the same build, the
same data layout. (No power-draw figure is claimed — none measured.)

> **Operator step honesty:** installing the launchd service on macOS, signing
> freshly-copied binaries, and Docker volumes are manual steps. The
> authoritative runbook is [`docs/deployment.md`](../deployment.md) and
> [`docs/docker.md`](../docker.md); this page is the 60-second summary.

## Bare metal (macOS / Linux)

```sh
# 1. Build the release binaries (server + brain CLI + mcp + bench).
cargo build --release --features bench \
  --bin brain-server --bin brain --bin mcp --bin bench

# 2. Install the launchd service + copy the CLI binaries to ~/.local/bin.
#    This also strips the macOS com.apple.provenance xattr that otherwise
#    triggers Gatekeeper SIGKILL (exit 137) on first exec.
scripts/install-service.sh

# 3. Verify.
brain doctor
brain status
```

- Live DB: `~/.openclaw/workspace/brain.db` (override `BRAIN_DB_PATH`).
- Logs: `~/Library/Logs/brain-server.{log,err.log}`.
- Auth: bearer token from `AUTH_TOKEN_FILE` (default off if none resolves).

## Docker

```sh
docker build -t brain-server .
docker run -p 8765:8765 -v "$HOME/.openclaw/workspace:/data" brain-server
```

See [`docs/docker.md`](../docker.md) for the image, env surface, and volume
layout.

## Next

[Quickstart](./quickstart.md) — a 5-minute run through recall, a proposal, and
an audit verify.
