#!/usr/bin/env python3
"""R51 gate-law census — two detectors, agreement required.

Detector (a) module boundary: the `#[cfg(test)]` IMMEDIATELY FOLLOWED BY
`mod tests {` — NOT the file's first `#[cfg(test)]`.
Detector (b) item attribute: the nearest preceding `#[cfg(test)]` on an item.

A site is production only if BOTH detectors say production.
Comment-stripped per the `handler_body` precedent (v1.28.86 / F7-07).
"""

import os
import re
import sys

CFG_TEST = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
MOD_TESTS = re.compile(r"^\s*(?:pub\s+)?mod\s+tests\b")
ATTR_LINE = re.compile(r"^\s*#\[")
NEEDLE = re.compile(r"LoopDriver\s*::\s*new\s*\(")


def strip_comments(src):
    """Line-wise comment stripper: // line comments and /* */ blocks.

    Not a full lexer. Deliberately conservative: it only ever REMOVES text,
    so it can under-report a site, never invent one. A missed site shows up
    as a census disagreement, not a silent production entry.
    """
    out, in_block = [], False
    for line in src.splitlines():
        res, i, n, in_str, in_chr = [], 0, len(line), False, False
        while i < n:
            two = line[i:i + 2]
            if in_block:
                if two == "*/":
                    in_block = False
                    i += 2
                else:
                    i += 1
                continue
            if not in_str and not in_chr and two == "/*":
                in_block = True
                i += 2
                continue
            if not in_str and not in_chr and two == "//":
                break
            c = line[i]
            if c == '"' and not in_chr:
                in_str = not in_str
            elif c == "'" and not in_str and line[i:i + 3] != "'\\":
                in_chr = not in_chr
            elif c == "\\" and (in_str or in_chr):
                res.append(c)
                i += 1
                if i < n:
                    res.append(line[i])
                    i += 1
                continue
            res.append(c)
            i += 1
        out.append("".join(res))
    return out


def brace_span(lines, start_idx, from_col=0):
    """Return 1-based end line of the block whose `{` is at/after from_col."""
    depth, seen = 0, False
    for idx in range(start_idx, len(lines)):
        line = lines[idx]
        for col in range(from_col if idx == start_idx else 0, len(line)):
            c = line[col]
            if c == "{":
                depth += 1
                seen = True
            elif c == "}":
                depth -= 1
                if seen and depth == 0:
                    return idx + 1
        if seen and line.rstrip().endswith(";") and depth == 0:
            return idx + 1  # no braces: an attribute-carrying statement
    return len(lines)


def detector_a_test_range(lines):
    """(start, end) of the `#[cfg(test)]` immediately followed by `mod tests {`."""
    for i, line in enumerate(lines):
        if not CFG_TEST.search(line):
            continue
        j = i + 1
        while j < len(lines) and not lines[j].strip():
            j += 1
        if j < len(lines) and MOD_TESTS.match(lines[j]):
            return j + 1, brace_span(lines, j, len(lines[j]) - len(lines[j].lstrip()))
    return None


def detector_b_test_ranges(lines):
    """Spans of every item carrying a `#[cfg(test)]` attribute."""
    spans, i, n = [], 0, len(lines)
    while i < n:
        if not ATTR_LINE.match(lines[i]):
            i += 1
            continue
        start, is_test = i + 1, False
        while i < n and (ATTR_LINE.match(lines[i]) or not lines[i].strip()):
            if CFG_TEST.search(lines[i]):
                is_test = True
            i += 1
        if not is_test or i >= n:
            continue
        end = brace_span(lines, i, len(lines[i]) - len(lines[i].lstrip()))
        spans.append((start, end))
        i = max(i, end - 1)
    return spans


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "src"
    production, test_sites = [], []
    for dirpath, _, names in os.walk(root):
        for name in sorted(names):
            if not name.endswith(".rs"):
                continue
            path = os.path.join(dirpath, name)
            with open(path, encoding="utf-8") as fh:
                lines = strip_comments(fh.read())
            a = detector_a_test_range(lines)
            b = detector_b_test_ranges(lines)
            for idx, line in enumerate(lines):
                if not NEEDLE.search(line):
                    continue
                ln = idx + 1
                in_a = a is not None and a[0] <= ln <= a[1]
                in_b = any(s <= ln <= e for s, e in b)
                # production only if BOTH detectors agree
                (test_sites if (in_a or in_b) else production).append((path, ln))

    print("PRODUCTION (both detectors agree): %d" % len(production))
    for path, ln in production:
        print("  %s:%d" % (path, ln))
    print("TEST (either detector): %d" % len(test_sites))
    for path, ln in test_sites:
        print("  %s:%d" % (path, ln))


if __name__ == "__main__":
    main()
