#!/usr/bin/env bash
# Golden-output regression gate (performance-plan.md §5.1).
#
#   scripts/golden.sh capture        # before a change: write golden/
#   scripts/golden.sh check          # after a change: re-run and diff
#
# Any diff means the "optimization" changed behavior. Byte-identical or it's wrong.
set -euo pipefail

cd "$(dirname "$0")/.."
BIN=${BIN:-target/release/sqdist}
LIST=${LIST:-corpus/pypi-names.txt}
GOLDEN=${GOLDEN:-golden}

[ -x "$BIN" ] || { echo "no $BIN — cargo build --release" >&2; exit 2; }
[ -f "$LIST" ] || { echo "no $LIST — scripts/fetch-corpus.sh" >&2; exit 2; }

# Diverse on the axes that change code paths: length, charset, delimiters, hit density.
QUERIES=(
  requests            # short ascii, dense near-misses
  tensorflow          # medium ascii
  scikit-learn        # delimiters -> --pypi normalize path
  Django              # uppercase
  py                  # 2 chars, degenerate length prune
  aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa   # 30 chars, DP-heavy
  zzq7f3xk9wv2b       # near-zero hits, pure reject path
  urllib3             # digits
  rеquests            # Cyrillic е: homoglyph / keyboard_distance NA path
  рython              # Cyrillic р
)

run_all() {
  local out=$1 q safe
  mkdir -p "$out"
  for q in "${QUERIES[@]}"; do
    safe=$(printf '%s' "$q" | tr -c 'A-Za-z0-9._-' '_')
    # Full panel over the whole corpus: too big to store, so gate on its hash.
    "$BIN" --string "$q" --list "$LIST" \
      | shasum -a 256 | awk -v k="$safe" '{print k"  "$1}' >> "$out/full.sha256"
    # Same run filtered to the interesting tail: small, and diffs readably when the hash breaks.
    "$BIN" --string "$q" --list "$LIST" -t 2 > "$out/$safe.near.jsonl" || true
    "$BIN" --string "$q" --list "$LIST" --typosquat > "$out/$safe.typo.jsonl" || true
    "$BIN" --string "$q" --list "$LIST" --typosquat --pypi > "$out/$safe.typo-pypi.jsonl" || true
  done
  sort -o "$out/full.sha256" "$out/full.sha256"
}

case "${1:-}" in
  capture)
    rm -rf "$GOLDEN"; run_all "$GOLDEN"
    echo "captured into $GOLDEN/"
    ;;
  check)
    [ -d "$GOLDEN" ] || { echo "no $GOLDEN/ — run: $0 capture" >&2; exit 2; }
    tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
    run_all "$tmp"
    if diff -r "$GOLDEN" "$tmp"; then echo "OK: output unchanged"; else echo "FAIL: output changed" >&2; exit 1; fi
    ;;
  *) sed -n '2,8p' "$0"; exit 2 ;;
esac
