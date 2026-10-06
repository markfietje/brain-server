# Lesson 1: Installing and configuring

**Level:** L2 · **Time:** about 25 minutes · **You need a terminal and a build toolchain**

## What you will do

Get the server installed, running, and configured the way your team works.
This lesson covers one machine, the default setup. Docker and reference
architectures have their own pages, linked at the end.

## Build and first run

```bash
git clone <your copy>
cd brain-server
cargo build --release --features bench --bin brain-server --bin brain
./target/release/brain-server
```

The server starts on `127.0.0.1:8765` by default. That loopback-only default
is deliberate: out of the box, nothing outside the machine can reach it.
Check it answered:

```bash
curl -s localhost:8765/ready
brain doctor
```

`brain doctor` is the one health command worth memorizing. It checks the
service, the database, and readiness in one pass, and it can verify a
backup file at the same time.

## The honest answer about remote access

Sooner or later someone asks to reach the server from another machine. Here
is the exact truth, because the docs you may find disagree and the truth is
the one that matches the code:

- Binding a non-loopback address without the explicit opt-in env
  (`BIND_PUBLIC=1` style) produces a **warning, and the server binds
  anyway**. It does not refuse. Two cases DO refuse: an address that cannot
  be parsed, and a non-loopback bind with no token configured at all.
- Translation: do not rely on the bind setting as your security. Rely on
  the token, and on a reverse proxy if you need real network posture.
  Loopback plus a proxy is the blessed shape, and there is a page on
  proxy/SSO setups for exactly that.

## Install as a service (macOS)

```bash
scripts/install-service.sh
```

This installs the binaries, writes the launchd plist, creates the token
file, and strips the macOS quarantine attribute that would otherwise get
the freshly-copied binary killed on first run. After it, the server comes
back on boot:

```bash
launchctl list | grep brain
tail -f ~/Library/Logs/brain-server.log
```

Linux has a systemd unit page with the equivalent. Whatever the platform,
the operating rhythm you are signing up for is: the server restarts itself,
you never babysit it, and you read logs when something odd happens.

## The one setting that shapes everything: write posture

`BRAIN_WRITE_POSTURE` decides how writes behave:

- `review`: assistant and auto-capture writes wait in the queue for a
  person. This is what the installer provisions for new setups, and it is
  what Level 1 assumes.
- `open`: direct operator writes to the write endpoints insert immediately,
  screened but not gated. This is the compiled default, so a hand-rolled
  start with no configuration behaves this way.

Know which one your instance runs. It changes what your reviewers see, and
it is a one-line answer your auditors will ask for.

## First-run configuration, the guided way

```bash
brain setup            # interactive: pick a profile preset, bind it to a domain
brain setup --yes      # scriptable
```

Profiles bundle the knobs (posture, screening strictness, retention
defaults) into named starting points, so configuration is a choice among
reviewed options rather than twenty env vars from memory. Every knob that
exists, and its exact meaning, lives on the configuration reference page.
Two habits pay for themselves: change one knob at a time, and write down
why in your own ops log when a value is surprising.

## Token and file hygiene

The token file lands at `~/.config/brain-server/auth-token`, mode 0600,
meaning only your user reads it. Keep it that way. Secret files in this
system are expected to be private (owner-only), and several tools will
refuse a secret file that group or world can read. That refusal looks like
an inconvenience the first time and like a saved week the second.

## Exercise

1. Build and start the server. Confirm `brain doctor` is green.
2. Run `brain status` and note the version. You will compare it against
   the changelog every time you update, so find the changelog now.
3. Change nothing else. Stop the server, start it again, and confirm the
   memory from any earlier exercise survived. (It should. Data lives in
   the database file, not the process.)
4. Deliberately chmod the token file to 0644 and run a command that reads
   it. Read the refusal. Restore 0600.

## What you learned

- Default bind is loopback. The bind warns, the token protects.
- install-service.sh gives you a boot-persistent service and a 0600 token.
- WRITE_POSTURE review versus open, and why it shapes the whole team's day.
- brain setup for guided configuration; one knob at a time.
- Secret files must be 0600, and the system enforces it loudly.

## Next

[Lesson 2: Who gets in](02-who-gets-in.md)

Further reading: [Quickstart](../../quickstart.md),
[Configuration](../../configuration.md),
[Deployment](../../deployment.md),
[Linux systemd unit](../../systemd-service.md),
[Reverse proxy SSO](../../proxy-sso.md).
