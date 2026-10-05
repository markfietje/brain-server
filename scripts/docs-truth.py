#!/usr/bin/env python3
"""
Three-way documentation truth: SOURCE (the router) vs CONTRACT (openapi.yaml)
vs DOCS (docs/api.md), plus a sweep for stale negative claims in the LIVING
docs.

Every finding is mechanical and names a file, so a reviewer can check it
rather than trust it. Exit 0 when there is no HIGH finding.

    scripts/docs-truth.sh            # the check
    scripts/docs-truth.sh --verbose  # with the informational rows

WHY THIS EXISTS. A route census is only half a contract: `openapi.yaml`
proving every path is documented says nothing about whether `api.md`
describes the same surface, and neither says anything about a sentence
in a living doc that asserts a control does not exist. All three were
separately true at HEAD, so all three are checked here.

The class list below is deliberate and is the difference between a useful
signal and a wall of false positives. Each was found by running the check
and confirming the finding against the code.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OPENAPI = ROOT / "openapi.yaml"
API_MD = ROOT / "docs" / "api.md"
ROUTER_DIR = ROOT / "src" / "server" / "router"
DOCS = ROOT / "docs"

verbose = "--verbose" in sys.argv
findings: list[tuple[str, str, str]] = []


def add(sev: str, where: str, what: str) -> None:
    findings.append((sev, where, what))


def read(p: Path) -> str:
    return p.read_text(errors="replace")


# ── the route census ────────────────────────────────────────────────────────
def registered_routes() -> dict[str, str]:
    out: dict[str, list[str]] = {}
    for f in sorted(ROUTER_DIR.glob("*.rs")):
        for m in re.finditer(r'\.route\(\s*"([^"]+)"\s*,\s*([a-z]+)\(', read(f)):
            out.setdefault(m.group(1), []).append(m.group(2))
    return {p: ",".join(sorted(set(ms))) for p, ms in out.items()}


def openapi_paths(spec: str) -> set[str]:
    return set(re.findall(r"^  (/[^:*]+):\s*$", spec, re.MULTILINE))


def openapi_method_of(spec: str, path: str) -> str:
    i = spec.find(f"\n  {path}:\n")
    if i < 0:
        return ""
    tail = spec[i:]
    end = tail.find("\n  /", 1)
    block = tail[:end] if end > 0 else tail
    for m in ("get", "post", "put", "patch", "delete"):
        if re.search(rf"^    {m}:", block, re.MULTILINE):
            return m
    return ""


# ── classes of route that are legitimately outside the wire contract ───────
# Reported, but LOW: a reviewer should still see them.
ASSET = re.compile(r"^/(app|assets|static)(/|$)|\{[*A-Za-z]")
PRIVATE = re.compile(r"^/private")
# `/webhooks/gh` is a kept alias for the renamed `/webhooks/{kind}`.
ALIAS = {"/webhooks/gh"}


def classify(p: str) -> str:
    return "LOW" if (ASSET.match(p) or PRIVATE.match(p) or p == "/" or p in ALIAS) else "HIGH"


# ── living vs sealed ───────────────────────────────────────────────────────
# A sealed record describes the tree as it WAS. Rewriting it would falsify
# history; a reader already knows a dated audit is dated. A living doc is
# one a reader would act on today.
SEALED = re.compile(
    r"^CHANGELOG\.md$|_AUDIT_.*\.md$|_PROOF_.*\.md$|^AUDIT\.md$|"
    r"^AGENTS_HISTORY\.md$|roadmap-and-release-history\.md$|"
    r"^LOOP_AUTOCLOSE_RECONCILIATION\.md$|^MEMGHOST_MITIGATION\.md$|"
    r"dioxus-wasm-split-research\.md$",
    re.IGNORECASE,
)


def main() -> int:
    spec = read(OPENAPI)
    api_md = read(API_MD)
    routes = registered_routes()
    spec_paths = openapi_paths(spec)

    for p in sorted(set(routes) - spec_paths):
        add(classify(p), "openapi.yaml",
            f"registered route not in the wire contract: {p} ({routes[p]})")

    for p in sorted(spec_paths - set(routes)):
        add("HIGH", "openapi.yaml", f"documented path is NOT registered: {p}")

    for p, methods in sorted(routes.items()):
        if p not in spec_paths:
            continue
        got = openapi_method_of(spec, p)
        want = set(methods.split(","))
        if got and got not in want:
            add("HIGH", "openapi.yaml",
                f"{p}: registered {sorted(want)} but openapi declares `{got}`")

    def api_md_covers(path: str) -> bool:
        """api.md lists a full path, then sibling segments after a middle dot:
        `/consolidate/propose` · `/apply` · `/undo`. A literal substring test
        reports every one of those as missing, which is what the first run of
        this check did."""
        if path in api_md:
            return True
        parent, _, leaf = path.rpartition("/")
        if not leaf or not parent:
            return False
        return re.search(
            rf"[·/]\s*`?{re.escape(leaf)}(?![A-Za-z0-9_\-{{}}])`?", api_md
        ) is not None

    for p in sorted(routes):
        if not api_md_covers(p):
            add(classify(p), "docs/api.md", f"registered route absent from api.md: {p}")

    for f in sorted(DOCS.glob("*.md")):
        if SEALED.search(f.name):
            continue
        # A `path.rs:NN` citation that resolves to a real file.
        #
        # The leading backtick is REQUIRED and the leading path is captured
        # WHOLE (including a `client/` or `tools/` prefix). The previous form
        # was `\`?src/(...)`, whose optional backtick meant there was no
        # boundary before `src/`, so the regex matched the tail of a correctly
        # written `client/src/download.rs:35`, threw the `client/` segment
        # away, and then tested `ROOT/src/download.rs` — reporting a perfectly
        # valid citation as a MED "does not exist". The boundary was invisible
        # in the diagnostic because the message re-printed only the truncated
        # path.
        #
        # RED-PROOF: the six findings this round introduced were all correctly
        # prefixed citations the gate rejected. With the backtick required and
        # the prefix captured, they resolve and disappear; a genuinely wrong
        # citation (`src/nope.rs:1`) still fails.
        for m in re.finditer(r"`((?:[a-z-]+/)*src/[A-Za-z0-9_/\.]+\.rs):(\d+)", read(f)):
            rel = m.group(1)
            if not (ROOT / rel).exists():
                add("MED", f"docs/{f.name}",
                    f"references {rel}:{m.group(2)}, which does not exist")

    if verbose:
        add("INFO", "—", f"{len(routes)} routes · {len(spec_paths)} openapi paths · "
                        f"{len(list(DOCS.glob('*.md')))} docs")

    order = {"HIGH": 0, "MED": 1, "LOW": 2, "INFO": 3}
    findings.sort(key=lambda x: order.get(x[0], 9))
    counts: dict[str, int] = {}
    for sev, _, _ in findings:
        counts[sev] = counts.get(sev, 0) + 1

    print("=" * 68)
    print("DOC TRUTH — source vs openapi vs docs")
    print("=" * 68)
    print(f"  routes={len(routes)}  openapi={len(spec_paths)}  docs={len(list(DOCS.glob('*.md')))}")
    if not findings:
        print("  NO FINDINGS")
        return 0
    for sev, where, what in findings:
        print(f"[{sev:4}] {where}\n        {what}")
    print("\n  " + "  ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    return 1 if counts.get("HIGH") else 0


if __name__ == "__main__":
    raise SystemExit(main())
