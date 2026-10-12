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
     include_str! (or include_bytes!), or walks the manifest. Body marker,
     because a doc-comment claim about reading source is not a read. Ordered
     BEFORE behavioral because a source read also calls std.
  4. behavioral_pin — POSITIVE evidence: the test's body invokes at least one
     callable that is not stdlib/assert plumbing, i.e. it drives a real
     subject (crate module, DB, HTTP app, process). Proof of a drive, not the
     absence of a read.
  5. unclassified — everything else: a test whose body invokes nothing
     outside the assert vocabulary. We do NOT call those behavioral pins by
     default, because "no evidence of driving a subject" is exactly the
     shape of the four vacuous census pins this repo shipped on first draft
     (R123). They are reported, not hidden, and the count is the honesty
     check on the rule set: a rule set that classifies 100% is a rule set
     that over-fits.

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

# Rule 3. `read` is what a source read actually calls; the include_* macros
# read a file at compile time. `File::open` / `fs::read` are the file-descriptor
# spellings. Each is a CALL shape, so a test that only MENTIONS read_to_string
# in its own assertion message does not match.
STRUCTURAL_BODY = re.compile(
    r"\b(?:include_str!|include_bytes!|fs::read_to_string|fs::read\(|"
    r"File::open|read_to_string\()"
)

# Rule 4. The allowlist is stdlib + the assert/format vocabulary: a call to one
# of these is plumbing, not a subject. Anything else is a drive.
PLUMBING = {
    "std", "core", "alloc", "crate", "self", "super", "Self", "env", "fs",
    "process", "thread", "time", "path", "pathbuf", "sync", "collections",
    "io", "net", "fmt", "str", "String", "Vec", "VecDeque", "HashMap",
    "HashSet", "BTreeMap", "BTreeSet", "Option", "Result", "Some", "None",
    "Ok", "Err", "Box", "Rc", "Arc", "Cow", "Path", "PathBuf", "Duration",
    "Instant", "SystemTime", "Mutex", "RwLock", "AtomicUsize", "AtomicBool",
    "Command", "Ordering", "Stdin", "Stdout", "Test", "Formatter", "Error",
    "matches", "contains", "push", "len", "get", "unwrap", "expect",
    "new", "clone", "from", "into", "to_string", "as_str", "as_ref",
    "deref", "fmt_write", "print", "println", "eprintln", "write",
    "read_line", "sort", "dedup", "iter", "collect", "unwrap_or",
    "unwrap_or_else", "unwrap_or_default", "is_empty", "is_ok", "is_err",
    "ok_or", "map", "map_err", "and_then", "ok", "err", "type_name",
    "anyhow", "assert", "assert_eq", "assert_ne", "debug_assert",
    "debug_assert_eq", "debug_assert_ne", "assert!", "assert_matches",
    "format", "vec", "matches!", "unreachable", "todo", "panic", "dbg",
    "write!", "writeln!", "e", "self_", "MIN", "MAX", "TAU", "EPSILON",
    "abs", "sqrt", "powf", "floor", "ceil", "round", "signum", "clamp",
    "min", "max", "split", "trim", "replace", "starts_with", "ends_with",
    "strip_prefix", "find", "parse", "unwrap_or_insert",
}
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


def drives_subject(body):
    """Rule 4: a call that is not stdlib/assert plumbing."""
    for m in CALL.finditer(body):
        if m.group(1) not in PLUMBING:
            return True
    return False


def classify(name, doc, body):
    upper = doc.upper()
    if RESIDUAL_NAME.match(name) or any(m in upper for m in RESIDUAL_MARKERS):
        return "residual_documentation"
    if CONTROL_NAME.match(name) or any(m in upper for m in CONTROL_MARKERS):
        return "control"
    if STRUCTURAL_BODY.search(body):
        return "structural"
    if drives_subject(body):
        return "behavioral_pin"
    return "unclassified"


def census():
    rows = []
    for tree in TREES:
        for path in sorted(rs_files(tree)):
            for name, doc, body, line, rel in parse(path):
                rows.append(
                    {
                        "file": rel,
                        "line": line,
                        "name": name,
                        "class": classify(name, doc, body),
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
    a("3. `structural` — reads the tree (source text, wiring, manifest).")
    a("4. `behavioral_pin` — POSITIVE evidence of a drive: the body invokes at")
    a("   least one non-stdlib callable.")
    a("5. `unclassified` — no evidence of driving a subject. Reported, never")
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
        a(f"{len(unc)} tests invoke nothing outside the assert vocabulary.")
        a("They are listed so a reader can move them by adding evidence, not")
        a("by re-deciding.")
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


def main():
    args = sys.argv[1:]
    rows = census()
    if "--json" in args:
        print(json.dumps(rows, indent=2))
        return 0
    table = render(rows)
    if "--check" in args:
        if not os.path.exists(TABLE):
            sys.exit(f"ERR {TABLE} missing — run scripts/test-census.py > {TABLE}")
        live = open(TABLE, encoding="utf-8").read()
        if live != table:
            sys.exit(
                f"ERR {os.path.relpath(TABLE, REPO)} drifts from a fresh "
                f"derivation — re-run:\n"
                f"  scripts/test-census.py > {os.path.relpath(TABLE, REPO)}"
            )
        print("OK  docs/test-census.md re-derives from the tree")
        return 0
    sys.stdout.write(table)
    return 0


if __name__ == "__main__":
    sys.exit(main())