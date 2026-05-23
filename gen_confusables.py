#!/usr/bin/env python3
"""Generate src/confusables_data.rs from Unicode UTS#39 confusables.txt.

Usage:
    curl -sSL https://www.unicode.org/Public/security/latest/confusables.txt -o confusables.txt
    python3 gen_confusables.py
"""
import os
import sys

SRC = sys.argv[1] if len(sys.argv) > 1 else "confusables.txt"
OUT = os.path.join("src", "confusables_data.rs")

mappings = {}
with open(SRC, encoding="utf-8") as f:
    for line in f:
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
    out.write("// Auto-generated from Unicode UTS#39 confusables.txt by gen_confusables.py.\n")
    out.write("// Source code point -> confusable skeleton string. Sorted by code point.\n")
    out.write(f"// {len(items)} entries. DO NOT EDIT BY HAND.\n")
    out.write("pub static CONFUSABLES: &[(u32, &str)] = &[\n")
    for cp, sk in items:
        out.write(f'    (0x{cp:04X}, "{esc(sk)}"),\n')
    out.write("];\n")

print(f"wrote {len(items)} entries to {OUT}")
