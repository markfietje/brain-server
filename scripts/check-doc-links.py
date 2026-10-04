#!/usr/bin/env python3
"""Resolve every relative markdown link under docs/ against the filesystem.

Anchors and URLs are skipped; only relative paths ending in .md are checked.
Run from the repo root: python3 scripts/check-doc-links.py
"""
import pathlib
import re
import sys

DOCS = pathlib.Path("docs")
LINK = re.compile(r"\]\((\.{0,2}/[^)#]*?\.md)(?:#[^)]*)?\)")

broken: list[str] = []
checked = 0

for md in sorted(DOCS.rglob("*.md")):
    for target in LINK.findall(md.read_text(encoding="utf-8", errors="replace")):
        checked += 1
        resolved = (md.parent / target).resolve()
        if not resolved.is_file():
            broken.append(f"{md}: {target}")

print(f"checked {checked} relative .md links under {DOCS}/")
if broken:
    print(f"\nBROKEN ({len(broken)}):")
    for line in broken:
        print(f"  {line}")
    sys.exit(1)
print("all resolve")