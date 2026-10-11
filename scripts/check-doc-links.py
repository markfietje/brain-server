#!/usr/bin/env python3
"""Resolve every relative markdown link under docs/ against the filesystem.

Anchors and URLs are skipped; only relative paths ending in .md are checked.
Run from anywhere: python3 scripts/check-doc-links.py (the docs root is file-anchored)
"""
import pathlib
import re
import sys

# Anchored on THIS FILE's location, not the cwd: invoked from any directory
# (a runner, a hook, a shell sitting in shell/) must resolve the same tree.
# The cwd anchor was the register's F4-17 defect — from a foreign cwd the
# checker scanned nothing and exited 0.
DOCS = pathlib.Path(__file__).resolve().parent.parent / "docs"
LINK = re.compile(r"\]\((\.{0,2}/[^)#]*?\.md)(?:#[^)]*)?\)")

broken: list[str] = []
checked = 0

for md in sorted(DOCS.rglob("*.md")):
    for target in LINK.findall(md.read_text(encoding="utf-8", errors="replace")):
        checked += 1
        resolved = (md.resolve().parent / target).resolve()
        if not resolved.is_file():
            broken.append(f"{md}: {target}")

print(f"checked {checked} relative .md links under {DOCS}/")
if broken:
    print(f"\nBROKEN ({len(broken)}):")
    for line in broken:
        print(f"  {line}")
    sys.exit(1)
print("all resolve")