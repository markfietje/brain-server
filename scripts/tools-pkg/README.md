# StewardMem Tools

The local memory kernel and its command-line companions, in one signed
image. Everything inside is Developer ID signed and notarized; the image
itself is stapled, so the verdict is offline and permanent.

## What each tool does

| Tool | What it is |
|---|---|
| `brain-server` | The memory kernel — a local HTTP service on 127.0.0.1:8765 |
| `brain` | The CLI — store, recall, review, forget |
| `mcp` | The MCP server — the kernel's tools for your coding agent |
| `bench` | The benchmark harness |
| `brain-connector-stub` | The connector test double |
| `brain-connector-gh` | The GitHub connector |
| `brain-connector-crm` | The CRM connector |

## Verify first (everything, against this file's siblings)

```
shasum -a 256 --check SHA256SUMS
```

Every file on this image — the pkg, the uninstaller, every binary — is
covered by `SHA256SUMS`. A file that fails the check did not come from us.

## Install

**The normal path:** double-click **Install StewardMem Tools.pkg**. Core is
pre-selected; Bench and the Connectors are one click away. The installer
writes only inside your home folder, needs no administrator password, and —
if you already run a brain-server service — installs the new binaries
BESIDE yours (never over them) and tells you exactly what it kept.

**Scriptable:**

```
installer -pkg "Install StewardMem Tools.pkg" -target CurrentUserHomeDirectory
```

(add `-applyChoiceChangesXML` to select components; the log carries the
installer's full hand-off receipt.)

**No install at all:** every binary runs straight from this image
(`binaries/brain-server --help`). Nothing here needs to be installed to be
used.

## Uninstall

The pkg's installer writes two things: `~/Library/Application Support/
StewardMem/tools/<version>/` (the real binaries) and symlinks in
`~/.local/bin` pointing at them. The escape hatch removes exactly those:

```
bash uninstall-stewardmem-tools.sh [--dry-run]
```

It removes only what the installer created, refuses while your service
still runs from its anchor, and never touches your own files.

## Provenance

These binaries are the SIGNED bytes of the `v1.29.38` release (the same
build pass that shipped it — never rebuilt beside it). The signing pass
regenerated `SHA256SUMS` over the signed bytes; the release page carries
the same set. `brain-server --version` and `brain --version` name the
release; the staged set's hashes are recorded in the release's signing
receipt.
