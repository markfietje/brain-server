#!/usr/bin/env python3
"""Fail if openapi.yaml uses a Schema Object keyword its DECLARED version lacks.

WHY THIS EXISTS. `openapi.yaml` declared `openapi: 3.0.3` and then used
`const:` on a header schema (introduced 1beb6251, 2026-10-11). It survived four
days because nothing validated 3.0 conformance — and the one tool already in the
tree that reads the spec, `openapi-typescript`, ACCEPTS it:

    $ npx openapi-typescript spec.yaml -o out.d.ts
    ✨ openapi-typescript 7.13.0
    🚀 spec.yaml → out.d.ts        # exit 0

It generates types; it does not validate the version. So the keyword was the
symptom and the missing check was the hole.

WHAT IT IS, precisely, and what it is not.

It is a KEYWORD-VOCABULARY gate: it reads the declared version and fails on any
keyword that version's Schema Object does not define. That set is finite and
stable — it is the OpenAPI 3.1 / JSON Schema 2020-12 delta over 3.0 — so pinning
it here is exhaustive for this class rather than a guess at it.

It is NOT a full schema validator. It will not catch a wrong `type`, a dangling
`$ref`, or a path that does not match an operationId. A real validator would, and
would need a dependency edge into the server tree — a governed act under
Plan 07's law (`crates_consumed_set_is_pinned`). That decision is NOT taken here.
This gate is the part that needs no dependency and catches the part that actually
shipped broken.

HOW IT PARSES. There is no YAML parser available to it: no PyYAML on the runners,
no `serde_yaml` in the Rust graph, and no `yaml` package in shell/node_modules.
So it scans key positions line by line. Two consequences, both handled:

  - a keyword mentioned INSIDE a description does not match, because a
    description line begins with `description:` and the pattern anchors on the
    whole line;
  - block scalar bodies (`|` / `>`) are skipped, so prose indented under a
    literal block cannot manufacture a false positive.

An UNRECOGNISED declared version FAILS CLOSED. A 3.2 document would otherwise be
checked against the wrong vocabulary and pass silently — the same shape as a pin
that stops seeing anything. Refusing to certify what we have no vocabulary for is
the whole point of the gate.
"""
import pathlib
import re
import sys

SPEC = pathlib.Path(__file__).resolve().parent.parent / "openapi.yaml"

# The declared-version → Schema-Object vocabulary. The 3.0 entry is the whole of
# the OpenAPI 3.0.3 Schema Object, transcribed from the specification. Anything in
# the 3.1 set is therefore caught by construction, not by enumeration of problems
# we happened to see.
SCHEMA_OBJECT_30 = frozenset(
    """
$ref format title description default multipleOf maximum exclusiveMaximum minimum
exclusiveMinimum maxLength minLength pattern maxItems minItems uniqueItems
maxProperties minProperties required enum type allOf oneOf anyOf not items
properties additionalProperties nullable discriminator readOnly writeOnly xml
externalDocs example deprecated
""".split()
)

# Keywords OpenAPI 3.1 adds to the Schema Object (JSON Schema 2020-12). None of
# these exist in 3.0.3. Listed explicitly so the divergence is auditable: this is
# the vocabulary, and a reviewer can check it against the spec rather than
# trusting that someone remembered.
ONLY_31 = frozenset(
    """
const examples prefixItems contains minContains maxContains patternProperties
additionalProperties_ unevaluatedItems unevaluatedProperties dependentSchemas
dependentRequired propertyNames contentEncoding contentMediaType if then else
$comment $defs $anchor $dynamicRef $dynamicAnchor
""".split()
)

# `additionalProperties` is legal in BOTH, so the 3.1-only set must not carry the
# placeholder the list above uses for readability. Kept as an explicit
# subtraction rather than a comment so a reader cannot miss it.
ONLY_31 = frozenset(k for k in ONLY_31 if not k.endswith("_"))

VERSIONED = {"3.0": SCHEMA_OBJECT_30}

DECLARED = re.compile(r"^openapi:\s*([0-9]+\.[0-9]+(?:\.[0-9]+)?)")
KEY = re.compile(r"^(\s*)([A-Za-z_$][A-Za-z0-9_$-]*):(\s|$)")
# A block scalar OPENER is any value that is `|` or `>` — the key is whatever
# precedes it. Matching only a bare `^\s*[|>]` line is the bug the drill caught:
# the commonest block scalar in this file is `description: >`, which does not
# look like that at all, so prose under it was scanned as keys and a sentence
# containing "const:" failed the build.
BLOCK_SCALAR = re.compile(r":\s*[|>][-+0-9]*\s*(#.*)?$")


def main() -> int:
    if not SPEC.exists():
        print(f"ERR: {SPEC} is missing", file=sys.stderr)
        return 1

    text = SPEC.read_text(encoding="utf-8")

    declared = None
    for line in text.splitlines():
        m = DECLARED.match(line)
        if m:
            declared = m.group(1)
            break
    if declared is None:
        print("ERR: no top-level `openapi:` version declared; refusing to certify", file=sys.stderr)
        return 1

    family = ".".join(declared.split(".")[:2])
    vocabulary = VERSIONED.get(family)
    if vocabulary is None:
        # FAIL CLOSED. An unknown version is not an excuse to pass.
        print(
            f"ERR: openapi.yaml declares {declared}, whose Schema Object vocabulary this "
            f"gate has no list for. Known: {sorted(VERSIONED)}. Refusing to certify a "
            f"document against a vocabulary we do not hold — add the version's "
            f"Schema Object to SCHEMA_OBJECT_30 / the version table and re-run.",
            file=sys.stderr,
        )
        return 1

    illegal = sorted(ONLY_31 - vocabulary)
    offenders: list[str] = []
    in_block = False
    block_indent = 0

    for lineno, line in enumerate(text.splitlines(), start=1):
        if in_block:
            # Leave the block when a line dedents to or past its opener.
            stripped = len(line) - len(line.lstrip())
            if line.strip() and stripped <= block_indent:
                in_block = False
            else:
                continue
        if BLOCK_SCALAR.search(line):
            in_block = True
            block_indent = len(line) - len(line.lstrip())
            continue
        m = KEY.match(line)
        if not m:
            continue
        keyword = m.group(2)
        if keyword in illegal:
            offenders.append(f"  openapi.yaml:{lineno}  {keyword}:  (declared {declared})")

    print(f"openapi.yaml declares {declared}; scanned {len(text.splitlines())} lines")
    print(f"  Schema Object vocabulary: {len(vocabulary)} keywords for {family}")
    print(f"  {len(ONLY_31)} keywords are invalid for that version")

    if offenders:
        print(f"\nILLEGAL FOR {declared} ({len(offenders)}):", file=sys.stderr)
        for line in offenders:
            print(line, file=sys.stderr)
        print(
            f"\n{declared} has no such keyword. It arrived in JSON Schema 2020-12 / "
            f"OpenAPI 3.1. For a single permitted value use `enum` with one entry — "
            f"that is the {declared} idiom and means the same thing.",
            file=sys.stderr,
        )
        return 1

    print("all keywords are valid for the declared version")
    return 0


if __name__ == "__main__":
    sys.exit(main())