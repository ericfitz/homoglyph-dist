#!/usr/bin/env bash
# Layer A end-to-end benchmark matrix (performance-plan.md §1).
#
#   scripts/bench.sh [name]     # name labels the run, default: git sha
#
# Writes bench-out/<name>/*.json + *.md. Compare runs with:
#   diff <(jq -r '.results[]|"\(.command)\t\(.mean)"' bench-out/A/*.json) ...
set -euo pipefail

cd "$(dirname "$0")/.."
BIN=${BIN:-target/release/sqdist}
CORPUS=${CORPUS:-corpus}
NAME=${1:-$(git rev-parse --short HEAD)}
OUT="bench-out/$NAME"

[ -x "$BIN" ] || { echo "no $BIN — cargo build --release" >&2; exit 2; }
[ -f "$CORPUS/pypi-names.txt" ] || { echo "no corpus — scripts/fetch-corpus.sh" >&2; exit 2; }
mkdir -p "$OUT"

# 50k slice for the corpus-size axis (and for dhat runs).
[ -f "$CORPUS/pypi-50k.txt" ] || head -50000 "$CORPUS/pypi-names.txt" > "$CORPUS/pypi-50k.txt"

bench() { # bench <slug> <hyperfine args...>
  local slug=$1; shift
  echo "== $slug"
  # -i: exit 1 just means "zero rows matched" in batch mode, not a failure.
  hyperfine -i --warmup 1 --runs 5 \
    --export-json "$OUT/$slug.json" --export-markdown "$OUT/$slug.md" "$@"
}

# Query length: 4 / 8 / 20 / 30 chars, ASCII, full corpus.
bench length -L q "pyyy,requests,aaaaaaaaaaaaaaaaaaaa,aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
  "$BIN --string {q} --list $CORPUS/pypi-names.txt -t 1"

# Charset: ASCII vs mixed-script homoglyph (keyboard_distance NA branch, skeleton path).
bench charset -L q "requests,rеquests,python,рython" \
  "$BIN --string {q} --list $CORPUS/pypi-names.txt -t 1"

# Mode: reject-path work differs per mode.
bench mode \
  "$BIN --string requests --list $CORPUS/pypi-names.txt" \
  "$BIN --string requests --list $CORPUS/pypi-names.txt -t 1" \
  "$BIN --string requests --list $CORPUS/pypi-names.txt --fields equal" \
  "$BIN --string requests --list $CORPUS/pypi-names.txt --typosquat" \
  "$BIN --string requests --list $CORPUS/pypi-names.txt --typosquat --pypi"

# Corpus size: 50k / 879k / 4.39M — confirms linearity.
bench corpus -L f "pypi-50k.txt,pypi-names.txt,npm-names.txt" \
  "$BIN --string requests --list $CORPUS/{f} -t 1"

# Hit density: many near-misses vs near-zero hits (reject cost vs emit cost).
bench density -L q "requests,zzq7f3xk9wv2b" \
  "$BIN --string {q} --list $CORPUS/pypi-names.txt -t 2"

echo "wrote $OUT/"
