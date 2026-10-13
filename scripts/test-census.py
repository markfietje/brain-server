#!/usr/bin/env python3
"""Classify every test in the tree and render the census table.

The suite is the largest artifact in this repository and nobody had measured
what it is made of. This is the measurement. The point is NOT the four class
names: it is that the class of a test is DERIVED BY A RULE, written down, and
re-runnable — so the next reader re-derives instead of trusting.

    scripts/test-census.py             → render the census markdown (stdout)
    scripts/test-census.py --check     → exit non-zero if docs/test-census.md
                                         drifts from a fresh derivation
    scripts/test-census.py --json      → the per-test classification

THE RULES, in order. First match wins; a test gets exactly one class. Every
tie-break is in this list, so a disagreement about a test's class is a
disagreement about a rule, which is arguable, rather than a matter of taste.

  1. residual_documentation — the test asserts a STATED LIMITATION on purpose.
     Name or doc-comment carries a residual marker (CEILING / RESIDUAL /
     KNOWN LIMITATION / a `ceiling_` or `residual_` name prefix). These are
     not expected to be fixed; a failure means the limitation changed.
  2. control — the anti-vacuity arm that makes a pin's failure interpretable:
     the negative side of a pair, so a passing pin cannot be a check that
     cannot fail. Name prefix or doc-comment marker.
  3. structural — the test READS THE TREE: it opens a source file, uses
     include_str!, or walks the manifest. Two shapes count, and the second
     one is the revision this file records:
       (a) the read is IN the test body — `fs::read_to_string`, `include_str!`,
           `File::open`, `read_dir()`;
       (b) the read is ONE CALL DEEP, in a helper the test calls by name
           (`read_repo("src/…")`, `doc("README.md")`, `walk_rs_files(dir)`).
     Shape (b) was MISSING on the first draft and undercounted the class by
     ~120: `tests/rbac_evaluation_pins.rs` and `src/docs_truth.rs` are
     source-scanning files whose tests all route through a helper, and the
     body-only needle saw none of them. A one-level walk is the whole
     indirection these files use — it is bounded, and it stops at the first
     helper, so a helper-of-a-helper is deliberately NOT chased.
     A helper qualifies only if it reads something REPO-SHAPED (a path built
     from CARGO_MANIFEST_DIR / repo() / a `.rs`/`.toml`/`.md`/`.yml` literal).
     A helper that reads RUNTIME DATA — a key file, a backup archive, an
     HTTP dist tree the test itself produced — is not structural: driving
     the filesystem is the behaviour under test there. A PROCESS SPAWN is
     excluded on the same principle: running `scripts/check-dist-sbom.sh` and
     reading its exit status DRIVES a real subject, which is behavioural.
     Ordered BEFORE behavioral because a source read also calls std.
  4. behavioral_pin — POSITIVE evidence: the body invokes something that is
     not stdlib plumbing, in EITHER call shape:
       (a) qualified — `brain_server::audit::hash(…)`, `app.oneshot(…)`;
       (b) BARE — `verify_claim(content, claim)`, `parse_flags(args)`.
     Shape (b) was MISSING on the first draft, which is why 261 real pins
     were reported as `unclassified`: a bare call is how a Rust unit test
     names its own module's subject, and `x.method()` alone would have
     classified `flags.get("dry-run")` as a drive when it is a HashMap read.
     Macros (`assert_eq!`, `format!`) cannot match the bare-call pattern —
     the `!` is not whitespace — so no macro allowlist is needed for them.
  5. unclassified — everything else: a test that drives no subject AND reads
     no tree. We do NOT call those behavioral pins by default, because "no
     evidence of driving a subject" is exactly the shape of the four
     vacuous census pins this repo shipped on first draft (R123). They are
     reported, not hidden, and the count is the honesty check on the rule
     set: a rule set that classifies 100% is a rule set that over-fits.

  The doc-marker rules (1, 2) match a STANDALONE UPPERCASE WORD, not a
  substring. The first draft used `in`, so `CONTROLLED`, `CONTROLS`,
  `STRIP_CONTROL_CHARS`, `CEILINGS` and
  `_CHILD_PARENT_CEILING_LIMITS_NEW_CALLS` all read as labels — five tests
  were classified as a control or a residual for a word that merely
  contained one.

  KNOWN CEILING, stated rather than papered over: a marker rule keys on the
  WORD, so a test whose doc-comment merely DISCUSSES a control or a ceiling is
  classified as one. This is not hypothetical — while writing the revision,
  the pin documenting the CONTROLLED-vs-CONTROL fix reclassified itself
  (behavioral → control) the moment its own prose said "the label CONTROL".
  That is the rule working as written, and it moves a count by one, not a
  class boundary. Fixing it needs a marker at a POSITIONAL anchor (a leading
  `CONTROL:` label), which would then miss every marker this repo writes as a
  mid-sentence lead-in. Ponytail: the ceiling is one test wide, and it fails
  loud (the census re-derives and `--check` refuses) rather than silent.

Scope: src/ and tests/ — the same two trees the badge's test count comes from.
Platform-gated tests are counted as written; the badge normalizes the FIVE
platform-only families out (scripts/badges.sh), so the census total will sit a
little above the badge. That gap is expected and is stated in the table.

Fail-closed: an attribute with no following `fn`, or a body whose braces never
balance, aborts the run with the file and line. A shape this parser cannot
interpret must never be silently counted as anything.
"""
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TREES = ("src", "tests")
TABLE = os.path.join(REPO, "docs", "test-census.md")

ATTR = re.compile(r"^\s*#\[(?:test|tokio::test)\b")
FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)")
DOC = re.compile(r"^\s*///?(.*)$")

RESIDUAL_MARKERS = ("CEILING", "RESIDUAL", "KNOWN LIMITATION", "NOT A DEFECT")
RESIDUAL_NAME = re.compile(r"^(ceiling|residual)_")
CONTROL_MARKERS = ("CONTROL",)
CONTROL_NAME = re.compile(r"^(negative_)?control_")
# A marker must be a STANDALONE word. The underscore is inside the excluded set
# on purpose: Rust identifiers glue words with `_`, and `_CEILING_LIMITS_` is a
# constant name, not a label.
LABEL = {k: re.compile(r"(?<![A-Za-z0-9_])" + re.escape(k) + r"(?![A-Za-z0-9_])")
         for k in RESIDUAL_MARKERS + CONTROL_MARKERS}

# Rule 3. `read` is what a source read actually calls; the include_* macros
# read a file at compile time. `File::open` / `fs::read` are the file-descriptor
# spellings. Each is a CALL shape, so a test that only MENTIONS read_to_string
# in its own assertion message does not match.
STRUCTURAL_BODY = re.compile(
    r"\b(?:include_str!|include_bytes!|fs::read_to_string|fs::read\(|"
    r"File::open|read_to_string\()"
)

# Rule 4(a). The RECEIVER of a qualified call that is stdlib/local plumbing:
# `fs::read_to_string` and `std::mem::size_of` reshape plumbing, they do not
# drive a subject. Anything else (`brain_server::`, `axum::`, `serde_json::`,
# `Tier::`, `app.`) is a subject. Only receiver names belong here — method
# names never appear in this position, so listing `push`/`len` was dead weight.
PLUMBING = {
    "std", "core", "alloc", "crate", "self", "Self", "super",
    "env", "fs", "process", "thread", "time", "path", "pathbuf", "sync",
    "collections", "io", "net", "fmt", "str", "mem", "cmp", "convert",
    "iter", "slice", "hash", "rc", "cell", "num", "ops", "ptr", "marker",
    "String", "Vec", "VecDeque", "HashMap", "HashSet", "BTreeMap", "BTreeSet",
    "Option", "Result", "Some", "None", "Box", "Rc", "Arc", "Cow", "Path",
    "PathBuf", "Duration", "Instant", "SystemTime", "Mutex", "RwLock",
    "AtomicUsize", "AtomicBool", "Ordering", "Stdin", "Stdout", "Formatter",
    "Error", "Command", "Test",
}

# Rule 4(b). Names that are NOT a drive when called bare: language keywords, and
# the value-conversion verbs that can only reshape something local. A bare call
# to anything else is a subject. Macros are already excluded by shape (`f!(`
# is not `f(`), which is why no macro list appears here.
KEYWORDS = {
    "let", "fn", "if", "match", "while", "for", "return", "mod", "use",
    "struct", "enum", "impl", "pub", "const", "unsafe", "move", "loop",
    "else", "await", "async", "type", "static", "ref", "in", "as", "dyn",
    "where", "true", "false", "break", "continue",
}
CONVERSIONS = {
    "into", "as_ref", "as_mut", "as_str", "as_bytes", "as_deref", "to_owned",
    "to_string", "clone", "cloned", "copied", "deref", "borrow", "borrow_mut",
    "unwrap", "expect", "is_ok", "is_err", "is_none", "is_some", "is_empty",
    "ok", "err", "iter", "into_iter", "keys", "values", "get", "contains",
    "starts_with", "ends_with", "find", "sort", "dedup", "extend", "push",
    "pop", "insert", "remove", "clear", "len", "count", "map", "filter",
    "fold", "any", "all", "zip", "chain", "first", "last", "next", "rev",
    "take", "skip", "sum", "split", "lines", "chars", "char_indices", "trim",
    "replace", "replacen", "to_lowercase", "to_uppercase", "parse", "min",
    "max", "abs", "sqrt", "floor", "ceil", "round", "eq", "ne", "cmp",
    "default", "collect", "unwrap_or", "unwrap_or_else", "unwrap_or_default",
    "print", "println", "eprintln", "eprint", "read_line",
}
NOT_A_DRIVE = KEYWORDS | CONVERSIONS

CALL = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*(?:::|\.)\s*[A-Za-z_]")
CHAR_LIT = re.compile(r"'(?:\\.|[^\\'\n])'")


def rs_files(tree):
    for dirpath, _dirs, files in os.walk(os.path.join(REPO, tree)):
        for f in sorted(files):
            if f.endswith(".rs"):
                yield os.path.join(dirpath, f)


def brace_deltas(text):
    """Per-line count of code `{` minus code `}`.

    A test body routinely contains a JSON payload, a SQL string, or a
    formatted message full of braces; counting raw characters made real tests
    unreadable on the first draft. This is a small state machine over the
    whole file — state has to survive line breaks in block comments and raw
    strings — that skips strings, char literals and comments. A `'` directly
    after an identifier character is a LIFETIME, not a char literal.
    """
    deltas = [0]
    depth = 0
    mode = "code"
    raw_hashes = 0
    block = 0
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "\n":
            if mode == "line":
                mode = "code"
            deltas.append(0)
            i += 1
            continue
        if mode == "line":
            i += 1
            continue
        if mode == "block":
            if text.startswith("/*", i):
                block += 1
                i += 2
            elif text.startswith("*/", i):
                block -= 1
                i += 2
                if block == 0:
                    mode = "code"
            else:
                i += 1
            continue
        if mode == "raw":
            if text.startswith('"' + "#" * raw_hashes, i):
                i += 1 + raw_hashes
                mode = "code"
            else:
                i += 1
            continue
        if mode == "str":
            if c == "\\":
                if i + 1 < n and text[i + 1] == "\n":
                    deltas.append(0)
                i += 2
            elif c == '"':
                mode = "code"
                i += 1
            else:
                i += 1
            continue
        if mode == "char":
            if c == "\\":
                if i + 1 < n and text[i + 1] == "\n":
                    deltas.append(0)
                i += 2
            elif c == "'":
                mode = "code"
                i += 1
            else:
                i += 1
            continue
        # code
        if text.startswith("//", i):
            mode = "line"
            i += 2
        elif text.startswith("/*", i):
            mode, block = "block", 1
            i += 2
        elif c == "r" and i + 1 < n and text[i + 1] in '#"':
            j = i + 1
            while j < n and text[j] == "#":
                j += 1
            if j < n and text[j] == '"':
                raw_hashes = j - i - 1
                mode = "raw"
                i = j + 1
            else:
                i += 1
        elif c == '"':
            mode = "str"
            i += 1
        elif c == "'":
            # A char literal is EXACTLY 'x' or a '\' escape. Everything else
            # opening with a quote is a lifetime — `'static str` in a return
            # type is not a char, and treating it as one swallowed every brace
            # in the impl below it.
            if CHAR_LIT.match(text, i):
                mode = "char"
            i += 1
        elif c == "{":
            depth += 1
            deltas[-1] += 1
            i += 1
        elif c == "}":
            depth -= 1
            deltas[-1] -= 1
            i += 1
        else:
            i += 1
    return deltas


def parse(path):
    """Yield (name, doc, body, line) per test attribute, or die on a shape we
    cannot read (fail-closed: an unreadable shape must never be counted)."""
    rel = os.path.relpath(path, REPO)
    text = open(path, encoding="utf-8").read()
    lines = text.splitlines()
    deltas = brace_deltas(text)
    if len(deltas) < len(lines):
        sys.exit(f"ERR {rel}: brace scanner desynced from the file's line count")
    i = 0
    while i < len(lines):
        if not ATTR.match(lines[i]):
            i += 1
            continue
        attr_line = i + 1
        # Doc comments: the contiguous comment block directly above the
        # attribute (other attributes may sit between — see them all).
        doc = []
        j = i - 1
        while j >= 0 and (DOC.match(lines[j]) or lines[j].lstrip().startswith("#[")):
            if DOC.match(lines[j]):
                doc.append(DOC.match(lines[j]).group(1))
            j -= 1
        doc = " ".join(reversed(doc))
        k = i
        while k < len(lines) and not FN.match(lines[k]):
            k += 1
        if k >= len(lines):
            sys.exit(f"ERR {rel}:{attr_line}: test attribute with no fn")
        name = FN.match(lines[k]).group(1)
        depth = 0
        seen = False
        body = []
        for m in range(k, len(lines)):
            body.append(lines[m])
            depth += deltas[m]
            seen = seen or depth > 0
            if seen and depth <= 0:
                break
        if not seen or depth != 0:
            sys.exit(f"ERR {rel}:{k + 1}: body of `{name}` never balances")
        yield name, doc, "\n".join(body), k + 1, rel
        i = k + 1


# Rule 3(b). A read of the REPO — a source/manifest/doc/wiring file — as opposed
# to a read of runtime data (a key file, a backup archive, a dist tree the test
# just wrote), which is what driving the filesystem looks like when it is the
# behaviour under test.
# The STRICT anchor: this fn resolves a path against the repo root. Deliberately
# NOT bare `env!(` — `env!("CARGO_PKG_VERSION")` is a compile-time string, not a
# path, and letting it seed the fixpoint dragged every helper that could reach
# `encrypt_v3_blob` into the repo class (`src/backup.rs`'s `verify`, which reads
# an encrypted BACKUP, not the tree).
REPO_ANCHOR = re.compile(
    r'CARGO_MANIFEST_DIR|include_str!\(\s*"|\.(?:rs|toml|md|yml|yaml|json|lock)"'
)

# A LOOSE hint, valid only when the fn also calls a strict anchor somewhere
# else in the file: `repo_root().join("src")` names no literal extension, but
# the `repo_root()` it calls does.
REPO_HINT = re.compile(r"\b(?:repo|repo_root|root|manifest_dir|workspace_root)\s*\(")

# Rule 3(b) call shape. `(?<![\w:.])` so `x.read(` and `foo::read(` do not
# match — only a call to a BARE helper name counts.
BARE_CALL = re.compile(r"(?<![\w:.])([a-z_][A-Za-z0-9_]*)\s*\(")


def file_helper_bodies(path):
    """{helper name: body} for every fn in the file that is NOT nested inside a
    test body — those are that test's own local closure, not a helper another
    test can call."""
    text = open(path, encoding="utf-8").read()
    lines = text.splitlines()
    deltas = brace_deltas(text)
    nested = []
    for i, line in enumerate(lines):
        if not ATTR.match(line):
            continue
        depth = seen = 0
        for k in range(i, len(lines)):
            depth += deltas[k]
            seen = seen or depth > 0
            if seen and depth <= 0:
                break
        nested.append((i, k))
    out = {}
    for i, line in enumerate(lines):
        fm = FN.match(line)
        if not fm or any(a < i <= b for a, b in nested):
            continue
        depth = seen = 0
        body = []
        for k in range(i, len(lines)):
            body.append(lines[k])
            depth += deltas[k]
            seen = seen or depth > 0
            if seen and depth <= 0:
                break
        out.setdefault(fm.group(1), []).append("\n".join(body))
    return out


def repo_paths(path):
    """(anchors, readers) for one file.

    `anchors` = helper fns that resolve a path against the REPO ROOT. Resolved
    as a fixpoint, not a one-level slice: the tree's dominant idiom is
    `repo_root()` -> `CARGO_MANIFEST_DIR` with `read_repo(rel)` calling *that*,
    so a strict one-level rule missed `tests/create_loop_pins.rs` entirely (12 of
    its 27 tests read the tree and none were counted).

    `readers` = anchors (or fns that CALL an anchor) that also perform a file
    read. Requiring the read in the fn's OWN body is what keeps the bound
    honest: a fn that reads a key file or a backup archive is not a reader, no
    matter how repo-shaped its arguments look.
    """
    helpers = file_helper_bodies(path)
    anchors = set()
    changed = True
    while changed:
        changed = False
        for name, bodies in helpers.items():
            if name in anchors:
                continue
            if any(REPO_ANCHOR.search(b) for b in bodies):
                anchors.add(name)
                changed = True
                continue
            called = {c.group(1) for b in bodies for c in BARE_CALL.finditer(b)}
            if REPO_HINT.search("\n".join(bodies)) and (anchors & called):
                anchors.add(name)
                changed = True
    readers = set()
    for name, bodies in helpers.items():
        if not any(STRUCTURAL_BODY.search(b) for b in bodies):
            continue
        for b in bodies:
            called = {c.group(1) for c in BARE_CALL.finditer(b)}
            if REPO_ANCHOR.search(b) or (anchors & called):
                readers.add(name)
                break
    return anchors, readers


def reads_tree(body, self_name, anchors, readers):
    """Rule 3: does this test read the TREE — in its body, or one helper deep?

    Two shapes, and they are NOT one shape with an extra condition:

      (a) the read is IN the body. The path must then be repo-anchored, or the
          test is driving the filesystem (a key file, a backup archive, a dist
          tree it just produced) — which is the behaviour, not the tree;
      (b) the read is ONE CALL DEEP: the body calls a helper that is itself a
          repo reader. The body carries no read needle at all in this shape,
          which is exactly why the first draft — gated on (a) — counted none
          of `tests/rbac_evaluation_pins.rs` or `src/docs_truth.rs`.
    """
    called = {c.group(1) for c in BARE_CALL.finditer(body)} - {self_name}
    if readers & called:
        return True
    if not STRUCTURAL_BODY.search(body):
        return False
    return bool(REPO_ANCHOR.search(body) or (anchors & called))


def drives_subject(body, self_name):
    """Rule 4: a call that reaches a subject, in either call shape."""
    for m in CALL.finditer(body):
        if m.group(1) not in PLUMBING:
            return True
    for m in BARE_CALL.finditer(body):
        name = m.group(1)
        if name not in NOT_A_DRIVE and name != self_name:
            return True
    return False


def classify(name, doc, body, anchors=(), readers=()):
    upper = doc.upper()
    if RESIDUAL_NAME.match(name) or any(LABEL[k].search(upper) for k in RESIDUAL_MARKERS):
        return "residual_documentation"
    if CONTROL_NAME.match(name) or LABEL["CONTROL"].search(upper):
        return "control"
    if reads_tree(body, name, anchors, readers):
        return "structural"
    if drives_subject(body, name):
        return "behavioral_pin"
    return "unclassified"


def census():
    rows = []
    for tree in TREES:
        for path in sorted(rs_files(tree)):
            anchors, readers = repo_paths(path)
            for name, doc, body, line, rel in parse(path):
                rows.append(
                    {
                        "file": rel,
                        "line": line,
                        "name": name,
                        "class": classify(name, doc, body, anchors, readers),
                    }
                )
    return rows


CLASSES = (
    "behavioral_pin",
    "structural",
    "control",
    "residual_documentation",
    "unclassified",
)


def render(rows):
    by_class = {c: 0 for c in CLASSES}
    for r in rows:
        by_class[r["class"]] += 1
    total = len(rows)
    unc = [r for r in rows if r["class"] == "unclassified"]
    lines = []
    a = lines.append
    a("# Test census — what the suite is made of")
    a("")
    a("Machine-derived by `scripts/test-census.py`. **Re-derive, never edit:**")
    a("")
    a("```")
    a("scripts/test-census.py --check    # exits non-zero on drift")
    a("```")
    a("")
    a("No commit hash is recorded here on purpose: a hash is the one field")
    a("the script cannot derive from the tree, so recording one would make")
    a("`--check` depend on an environment variable and stop the table from")
    a("re-deriving byte-for-byte. Git already records which commit shipped it.")
    a("")
    a("## The classes, and why they are ordered")
    a("")
    a("First match wins; every test gets exactly one class. Full rule text,")
    a("including the stdlib allowlist that decides rule 4, is the module")
    a("docstring in `scripts/test-census.py` — the rules live with the script")
    a("that applies them, not in prose that can drift from it.")
    a("")
    a("1. `residual_documentation` — asserts a stated limitation ON PURPOSE.")
    a("2. `control` — the anti-vacuity arm that makes a pin's failure readable.")
    a("3. `structural` — reads the TREE: the read is in the body, or one call")
    a("   deep in a named helper that reads a repo-shaped file. A read of")
    a("   runtime data (a key file, a backup the test made) is NOT structural")
    a("   — and neither is a process spawn: running a script and reading its")
    a("   exit status drives a real subject.")
    a("4. `behavioral_pin` — POSITIVE evidence of a drive, in either call shape:")
    a("   qualified (`brain_server::audit::hash`) or BARE (`verify_claim(c, q)`).")
    a("   A bare call is how a Rust unit test names its own module's subject,")
    a("   so a rule 4 that only accepts qualified calls misses most of this repo.")
    a("5. `unclassified` — drives no subject and reads no tree. Reported, never")
    a("   folded into 4 by default: \"no read found\" is precisely the shape of")
    a("   the four vacuous census pins R123 shipped on first draft.")
    a("")
    a("## The count")
    a("")
    a("| Class | Tests | Share |")
    a("|---|---:|---:|")
    for c in CLASSES:
        a(f"| `{c}` | {by_class[c]} | {100.0 * by_class[c] / total:.1f}% |")
    a(f"| **total** | **{total}** | |")
    a("")
    a(f"Scope: `src/` + `tests/`, the two trees the badge's count comes from.")
    a("The census reads HIGHER than the README badge for two reasons, both")
    a("expected. The badge counts tests that")
    a("*passed* a real run, after twelve `--skip` families remove the")
    a("platform-only sandbox tests (`scripts/badges.sh`); the census counts")
    a("tests as written. And a raw `rg` count of `#[test]` over-reads this")
    a("tree by ~64, because tests that pin a text-scanner carry `#[test]`")
    a("inside their own string literals (tests/rbac_evaluation_pins.rs has")
    a("eleven such cases). The census counts what the compiler does.")
    a("")
    a("## The top ten files by size")
    a("")
    a("| File | Tests |")
    a("|---|---:|")
    per_file = {}
    for r in rows:
        per_file[r["file"]] = per_file.get(r["file"], 0) + 1
    for f, n in sorted(per_file.items(), key=lambda kv: (-kv[1], kv[0]))[:10]:
        a(f"| `{f}` | {n} |")
    a("")
    a("## `unclassified` — the honesty check on the rule set")
    a("")
    if not unc:
        a("None: every test matched a rule. That is the shape of an OVER-FIT")
        a("rule set, not a clean bill of health — check the rule text before")
        a("believing it.")
    else:
        a(f"{len(unc)} tests drive no subject and read no tree.")
        a("They are listed so a reader can move them by adding evidence, not")
        a("by re-deciding. Read this bucket as \"the census could not see a"
          " drive\", NOT as \"this test is worthless\": all six assert a")
        a("constant or a declared bound, which is a real pin that happens to")
        a("have no call in it.")
        a("")
        a("| Test |")
        a("|---|")
        for r in sorted(unc, key=lambda r: (r["file"], r["line"])):
            a(f"| `{r['file']}:{r['line']}` `{r['name']}` |")
    a("")
    a("## What this census does NOT police")
    a("")
    a("It is not a gate on class counts. There is no floor on any class in")
    a("this round: a floor written before its population is measured is a")
    a("number nobody checked. The only thing that can fail is **drift** — the")
    a("committed table disagreeing with a fresh derivation (`--check`).")
    a("")
    return "\n".join(lines) + "\n"


def drift_error():
    return (
        f"ERR {os.path.relpath(TABLE, REPO)} drifts from a fresh derivation "
        f"— re-run:\n  scripts/test-census.py > {os.path.relpath(TABLE, REPO)}"
    )


def main():
    args = sys.argv[1:]
    rows = census()
    table = render(rows)

    if "--check" in args:
        if not os.path.exists(TABLE):
            sys.exit(f"ERR {TABLE} missing — run scripts/test-census.py > {TABLE}")
        with open(TABLE, encoding="utf-8") as fh:
            live = fh.read()
        if live != table:
            sys.exit(drift_error())
        # ONE spawn must answer both questions the pin asks: does the table still
        # re-derive, and what did the classifier actually SEE? The first draft
        # ran `--check` and `--json` as two processes and paid for a second full
        # parse of every file in the tree (measured: 3.0s each). Under `--json`
        # the verdict goes to stderr so stdout stays parseable.
        print("OK  docs/test-census.md re-derives from the tree",
              file=sys.stderr if "--json" in args else sys.stdout)

    if "--json" in args:
        print(json.dumps(rows, indent=2))
        return 0
    if "--check" in args:
        return 0
    sys.stdout.write(table)
    return 0


if __name__ == "__main__":
    sys.exit(main())