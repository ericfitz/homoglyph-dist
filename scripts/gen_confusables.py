#!/usr/bin/env python3
"""Generate src/confusables_data.rs from Unicode UTS#39 confusables.txt.

Usage (from the repo root):
    curl -sSL https://www.unicode.org/Public/security/latest/confusables.txt -o confusables.txt
    python3 scripts/gen_confusables.py
"""
import os
import re
import sys

# Resolve paths relative to the repo root (scripts/'s parent) so the script
# works regardless of the current working directory.
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO_ROOT, "confusables.txt")
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(REPO_ROOT, "src", "confusables_data.rs")

# Read all lines first so we can scan the header comment block for version/date
# before the mapping-parsing loop strips comment text.
with open(SRC, encoding="utf-8") as f:
    raw_lines = f.readlines()

version = "unknown"
date = "unknown"
for raw_line in raw_lines:
    stripped = raw_line.strip()
    if not stripped.startswith("#"):
        break  # stop at first non-comment line
    # Match "# Version: 17.0.0" (possibly with trailing whitespace/text)
    m = re.search(r"Version:\s*([\d.]+)", stripped)
    if m:
        version = m.group(1)
    # Match "# Date: 2025-07-22" (date before optional comma and trailing text)
    m = re.search(r"Date:\s*(\d{4}-\d{2}-\d{2})", stripped)
    if m:
        date = m.group(1)

mappings = {}
for line in raw_lines:
    line = line.split("#", 1)[0].strip()
    if not line:
        continue
    parts = [p.strip() for p in line.split(";")]
    if len(parts) < 2:
        continue
    src_cps = parts[0].split()
    tgt_cps = parts[1].split()
    if len(src_cps) != 1:  # source is always a single code point in this file
        continue
    src = int(src_cps[0], 16)
    tgt = "".join(chr(int(c, 16)) for c in tgt_cps)
    mappings[src] = tgt

items = sorted(mappings.items())


def esc(s: str) -> str:
    out = []
    for ch in s:
        if ch == "\\":
            out.append("\\\\")
        elif ch == '"':
            out.append('\\"')
        elif ch == "\n":
            out.append("\\n")
        elif 0x20 <= ord(ch) < 0x7F:
            out.append(ch)
        else:
            out.append(f"\\u{{{ord(ch):x}}}")
    return "".join(out)


with open(OUT, "w", encoding="utf-8") as out:
    out.write(f"// Auto-generated from Unicode UTS#39 confusables.txt (v{version}, {date}).\n")
    out.write("// Source code point -> confusable skeleton string. Sorted by code point.\n")
    out.write(f"// {len(items)} entries.\n")
    out.write('/// Provenance of the embedded UTS#39 confusables data, surfaced by `-v`.\n')
    out.write(f'pub static CONFUSABLES_PROVENANCE: &str = "UTS#39 confusables.txt v{version} ({date})";\n')
    out.write("pub static CONFUSABLES: &[(u32, &str)] = &[\n")
    for cp, sk in items:
        out.write(f'    (0x{cp:04X}, "{esc(sk)}"),\n')
    out.write("];\n")

print(f"wrote {len(items)} entries to {OUT}")
