#!/usr/bin/env python3
"""Generate src/flowcrypt_data.rs from the FlowCrypt idn-homographs-database.

For each Basic-Latin (ASCII) base char B and each look-alike S in B's
similar_char list, emit (ord(S) -> uts39_skeleton(B)): the look-alike collapses
to the UTS#39 skeleton of its ASCII partner. UTS#39 always wins (entries where S
is itself a UTS#39 source are emitted but the Rust runtime merge skips them; the
generator logs skeleton disagreements). Dedup on S (lowest base wins), sort by S.

Provenance: the upstream repo has no releases, so we record the master commit
SHA (from the GitHub API, or --source-commit) + retrieval date into a
FLOWCRYPT_PROVENANCE constant for `-v`.

Pure stdlib. Usage:
  python3 scripts/gen_flowcrypt.py [--homographs <path|url>] [--confusables <path|url>]
                                   [--source-commit <sha>] [--source-date <YYYY-MM-DD>]
Accepts local paths or https URLs. FlowCrypt data is MIT (attributed in output).
"""
import datetime
import json
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "src" / "flowcrypt_data.rs"
HG_DEFAULT = "https://raw.githubusercontent.com/FlowCrypt/idn-homographs-database/master/homograph/homographs.json"
CONF_DEFAULT = "https://www.unicode.org/Public/security/latest/confusables.txt"
COMMIT_API = "https://api.github.com/repos/FlowCrypt/idn-homographs-database/commits/master"

def load(src):
    if src.startswith("http://") or src.startswith("https://"):
        with urllib.request.urlopen(src, timeout=120) as r:
            return r.read().decode("utf-8")
    return Path(src).read_text(encoding="utf-8")

def resolve_commit(explicit):
    """The idn-homographs-database has no releases, so provenance = master
    commit SHA + retrieval date. Use --source-commit if given, else query the
    GitHub API. Falls back to 'unknown' if the API is unreachable."""
    if explicit:
        return explicit
    try:
        data = json.loads(load(COMMIT_API))
        return data.get("sha", "unknown")[:7]
    except Exception:
        return "unknown"

def parse_uts39_skeletons(text):
    """code point (int) -> skeleton string, from confusables.txt lines
    'XXXX ; YYYY ZZZZ ; MA # ...' (single-source only, matching gen_confusables)."""
    sk = {}
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line or ";" not in line:
            continue
        parts = [p.strip() for p in line.split(";")]
        if len(parts) < 2:
            continue
        src_cps = parts[0].split()
        if len(src_cps) != 1:  # single-source only
            continue
        try:
            src = int(src_cps[0], 16)
            tgt = "".join(chr(int(c, 16)) for c in parts[1].split())
        except ValueError:
            continue
        sk[src] = tgt
    return sk

def uts39_skeleton_of(ch, sk):
    """Skeleton of a single ASCII base char under UTS#39 (or the char itself)."""
    return sk.get(ord(ch), ch)

def main():
    args = sys.argv[1:]
    hg = HG_DEFAULT
    conf = CONF_DEFAULT
    source_commit = None
    source_date = datetime.date.today().isoformat()
    i = 0
    while i < len(args):
        if args[i] == "--homographs":
            hg = args[i + 1]; i += 2
        elif args[i] == "--confusables":
            conf = args[i + 1]; i += 2
        elif args[i] == "--source-commit":
            source_commit = args[i + 1]; i += 2
        elif args[i] == "--source-date":
            source_date = args[i + 1]; i += 2
        else:
            print(f"unknown arg: {args[i]}", file=sys.stderr); sys.exit(2)
    commit = resolve_commit(source_commit)
    hgdb = json.loads(load(hg))
    sk = parse_uts39_skeletons(load(conf))

    # S codepoint -> (skeleton, base char) ; lowest base wins on dup.
    rows = {}
    conflicts = 0
    multi = 0
    for base, entry in hgdb.items():
        if len(base) != 1 or ord(base) >= 128:  # Basic-Latin base only
            continue
        target = uts39_skeleton_of(base, sk)
        for sim in entry.get("similar_char", []):
            ch = sim.get("char", "")
            if len(ch) != 1:
                continue
            cp = ord(ch)
            if cp < 128:  # don't remap ASCII to ASCII; UTS#39 owns ASCII
                continue
            # UTS#39 disagreement logging: if S already has a uts39 skeleton that
            # differs from our anchored target.
            if cp in sk and sk[cp] != target:
                conflicts += 1
            if cp in rows:
                multi += 1
                # keep lowest base char deterministically
                if base < rows[cp][1]:
                    rows[cp] = (target, base)
            else:
                rows[cp] = (target, base)

    items = sorted(rows.items())  # by codepoint
    lines = []
    lines.append("// Auto-generated from the FlowCrypt idn-homographs-database (MIT).")
    lines.append("// https://github.com/FlowCrypt/idn-homographs-database (homograph/homographs.json)")
    lines.append("// Look-alike code point -> UTS#39 skeleton of its ASCII partner.")
    lines.append("// Filtered to Basic-Latin base chars; anchored to UTS#39 (UTS#39 wins at runtime).")
    lines.append(f"// {len(items)} entries.")
    lines.append("")
    lines.append("/// Provenance of the embedded FlowCrypt data, surfaced by `-v`. The upstream")
    lines.append("/// repo has no releases, so we track the master commit SHA + retrieval date.")
    prov = f"FlowCrypt idn-homographs-database @ {commit} (retrieved {source_date})"
    lines.append("pub static FLOWCRYPT_PROVENANCE: &str =")
    lines.append(f'    "{prov}";')
    lines.append("")
    lines.append("pub static FLOWCRYPT: &[(u32, &str)] = &[")
    for cp, (target, _base) in items:
        esc = target.replace("\\", "\\\\").replace('"', '\\"')
        lines.append(f'    (0x{cp:04X}, "{esc}"),')
    lines.append("];")
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {OUT} ({len(items)} entries; commit {commit} {source_date}; {conflicts} uts39 disagreements logged; {multi} multi-base dups)", file=sys.stderr)

if __name__ == "__main__":
    main()
