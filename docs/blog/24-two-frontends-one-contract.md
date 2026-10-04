# Two frontends, one contract

*2026-10-04. We are shipping a second operator GUI while the first is still
served. This post is about the unglamorous part that makes that safe, and about
a documentation correction the second GUI forced us to make.*

Our documentation said we ship a Dioxus client: one Rust codebase, web, desktop,
iOS, and Android. That sentence was wrong, and it was wrong in two directions at
once.

We do ship that. It is the bundle the server serves at `/app` today, sixteen
panels deep, and the running service is pointed at it right now. But we also
ship something the docs never mentioned: a SvelteKit plus Tauri shell, with its
own CI workflow and its own Playwright suite, currently at eight routes and
being built toward replacing the first.

So the honest sentence is two sentences. The Dioxus client is what ships. The
shell is what we are building toward, and its own README freezes the old
client's removal until its parity gates pass. Writing "we have replaced the
Dioxus client with a Tauri shell" would have been the same class of error as the
one we were fixing, just pointed the other way. A reader who trusts that sentence
would open the wrong directory.

The correction also removed a claim I had been repeating for a while. The docs
said four platforms. `mobile` is a compile-smoke feature target. No store
submission has ever shipped. One page in our own documentation already said so
plainly while three others said otherwise, which is a useful reminder that a
single accurate sentence does not correct its neighbours.

## The hard part is not the second frontend

Writing a second frontend is ordinary work. Two of them reading one API is where
the interesting failures live, and both of ours are the kind that pass testing.

**The first is wire drift.** A frontend that hand-writes its types against a
careful reading of the API documentation will compile, and then disagree with
the server about a field name at runtime. The fix is unglamorous: the shell's
API client is generated from the kernel's `openapi.yaml`, and a test
regenerates it into a temporary directory and asserts **byte equality** against
the committed file. Not structural similarity. Not "does it still parse". The
exact bytes.

That choice is stricter than it needs to be, on purpose. A generator version
bump will fail the build even when the types are semantically identical. For a
security boundary, a surprising red build is much cheaper than a quiet change in
what the compiler believes the server said.

**The second is sanitizer drift, and it is worse.** We strip invisible Unicode
at five independent boundaries: the server, the shell, the OpenClaw plugin, the
Dioxus client, and a fixture. When five implementations each decide on their
own which characters are invisible, they diverge. Someone adds the bidi
isolates to the server set because a smuggling class needs them. The others keep
the old set. Every implementation still has tests. Every test is green. The
boundary differs by tree, and nothing notices.

The fix is one JSON file of named code point ranges that all five read, plus an
exhaustive test on the server side that walks the entire scalar space and
compares rather than checking a list of interesting characters. Adding a class
is now an edit to one file instead of a coordinated commit across five trees.

Here is the part I would underline if I could underline anything. Our CI
workflow triggers on the shell's paths **and** on that shared fixture. Without
that second trigger, the fixture could be edited in a change touching no shell
file, the shell's tests would not run, and the drift would land anyway. The
tests were not the hard part. Wiring the trigger that makes them fire at the
right moment was.

## What this does not prove

The parity fixture pins **which characters are invisible**. It does not prove
any consumer does nothing else surprising with them afterward. A tree that
strips the canonical set and then reintroduces a directional override by some
other route passes every test described here.

The wire gate covers the shell's client. The plugin's MCP surface and the Dioxus
client do not consume that generated file. Their types are hand-written and
their drift is caught by route and contract tests, which is a real check and a
weaker one.

And removing characters is lossy. A legitimate string containing a zero-width
joiner, which several writing systems use, loses those characters on the render
path. Storage keeps the bytes verbatim, so this is a display transform and
nothing more, but it is a transform and worth naming.

## Why we are shipping two frontends at all

Because the second one is better for what the shell needs to be. A typed
generated client, a real desktop shell, a component library, and strict content
security policy defaults are all easier to get right in this stack than in the
other one. That is a reason to move. It is not a reason to pretend the move has
happened.

Meanwhile the test that matters most is the boring one: does the thing the server
actually serves match what the docs say it serves? For us that is a single
environment variable and one directory, which is as close to an answer as this
kind of question gets.

Full mechanism write-up in the research note on
[UI contract parity](./../research/15-ui-contract-parity.md).