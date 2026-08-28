#!/usr/bin/env bash
# Quality gate (performance-plan.md §5.2 recall + §5.3 precision proxy).
#
#   scripts/quality.sh capture       # record the baseline into quality/
#   scripts/quality.sh check         # re-run and diff against it
#
# golden.sh proves output is byte-identical for the queries it pins. This one
# guards the thing golden.sh cannot see: whether a *future* prune trades
# detection for speed. Two measurements, one baseline file:
#
#   recall    — the 143 labeled typosquat pairs, broken out by technique.
#               A prune that drops a whole technique shows up as one bad row.
#   precision — emitted-row counts over a sample of top-15k queries against
#               full PyPI. Not precision in the ML sense (the corpus has no
#               labels at that scale); it is an exact fingerprint of what the
#               tool emits at scale, so any change to it is deliberate or a bug.
#
# Recall runs each malicious name as --string against a haystack of the top-15k
# plus every target, per the plan. It is tempting to feed the pairs through
# --stdin instead, since --typosquat classifies each pair independently of the
# rest of the list — but --stdin never calls may_emit, and the list-mode emit
# gate is exactly what this is here to protect. Verified: it catches a
# deliberately narrowed gate; the --stdin form did not.
set -euo pipefail

cd "$(dirname "$0")/.."
BIN=${BIN:-target/release/sqdist}
PAIRS=${PAIRS:-corpus/typosquats.csv}
TOP=${TOP:-corpus/pypi-top15k.txt}
LIST=${LIST:-corpus/pypi-names.txt}
OUT=${OUT:-quality}
# Queries for the precision sweep. The full 15k is the ~2.7 CPU-hour workload;
# a prefix is a gate you will actually run. 100 keeps the whole script near
# golden.sh's ~2.5 min. Raise it (SWEEP=2000) for a release check — the counts
# are only comparable against a baseline captured at the same SWEEP.
SWEEP=${SWEEP:-100}

[ -x "$BIN" ] || { echo "no $BIN — cargo build --release" >&2; exit 2; }
for f in "$PAIRS" "$TOP" "$LIST"; do
  [ -f "$f" ] || { echo "no $f — scripts/fetch-corpus.sh" >&2; exit 2; }
done

mode=${1:-}
case "$mode" in
  capture) dest=$OUT ;;
  check)   dest=$(mktemp -d) ;;
  *) echo "usage: $0 capture|check" >&2; exit 2 ;;
esac
mkdir -p "$dest"
report="$dest/quality.txt"

# --- §5.2 recall, by technique ------------------------------------------------
# Emit "malicious<TAB>target" pairs, score them, and join the emitted rows back
# to the technique label. A pair counts as detected when --typosquat emits it at
# all (batch mode emits only likely_typosquat / possible_combosquat).
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tail -n +2 "$PAIRS" | awk -F, '{print $1"\t"$2}' > "$tmp/pairs.tsv"
tail -n +2 "$PAIRS" | awk -F, '{print $1"\t"$2"\t"$5}' > "$tmp/labels.tsv"

# The haystack: the top-15k plus every labeled target, so each target is
# present to be found among realistic distractors.
cat "$TOP" <(tail -n +2 "$PAIRS" | cut -d, -f2) | sort -u > "$tmp/haystack.txt"

# Score every labeled pair through list mode and write "input<TAB>match<TAB>
# classification" for the rows that found the intended target. Exit 1 just
# means "nothing matched", which is a real answer, not a failure. The keys come
# out in a fixed order per the output contract, so no JSON parser is needed;
# --pypi adds *_normalized keys between them, which the pattern skips.
score_pairs() { # score_pairs <out.tsv> [extra sqdist args...]
  local out=$1; shift
  : > "$out"
  while IFS=$'\t' read -r mal target; do
    "$BIN" --string "$mal" --list "$tmp/haystack.txt" --typosquat "$@" 2>/dev/null |
      sed -E 's/.*"input":"([^"]*)".*"match":"([^"]*)".*"classification":"([^"]*)".*/\1\t\2\t\3/' |
      awk -F'\t' -v t="$target" '$2 == t' >> "$out" || true
  done < "$tmp/pairs.tsv"
}

# Per-technique recall table plus the TOTAL line, from a scored .tsv.
recall_table() { # recall_table <hits.tsv>
  awk -F'\t' '
    NR==FNR { hit[$1 FS $2] = 1; next }
    { total[$3]++; if (($1 FS $2) in hit) got[$3]++ }
    END {
      for (t in total) {
        g = (t in got) ? got[t] : 0
        printf "%-14s %3d/%-3d  %6.1f%%\n", t, g, total[t], 100*g/total[t]
      }
    }
  ' "$1" "$tmp/labels.tsv" | sort
  awk -F'\t' '
    NR==FNR { hit[$1 FS $2] = 1; next }
    { n++; if (($1 FS $2) in hit) g++ }
    END { printf "\n%-14s %3d/%-3d  %6.1f%%\n", "TOTAL", g, n, 100*g/n }
  ' "$1" "$tmp/labels.tsv"
}

score_pairs "$tmp/hits.tsv"
score_pairs "$tmp/hits-pypi.tsv" --pypi

{
  echo "# sqdist quality gate"
  echo
  echo "## recall by technique (--typosquat)"
  echo
  recall_table "$tmp/hits.tsv"
  echo
  echo "## recall by technique (--typosquat --pypi)"
  echo
  echo "# PEP 503 folds case and delimiters, so it recovers pairs that differ"
  echo "# only there — and correctly drops 'delimiter' ones to same_project."
  echo
  recall_table "$tmp/hits-pypi.tsv"
  echo
  echo "## missed pairs (--typosquat, no normalizer)"
  echo
  awk -F'\t' '
    NR==FNR { hit[$1 FS $2] = 1; next }
    !(($1 FS $2) in hit) { printf "%-32s %-32s %s\n", $1, $2, $3 }
  ' "$tmp/hits.tsv" "$tmp/labels.tsv" | sort
  echo
  echo "## classification split of detected pairs (--typosquat)"
  echo
  cut -f3 "$tmp/hits.tsv" | sort | uniq -c | awk '{printf "%-24s %s\n", $2, $1}'

  # --- §5.3 precision proxy ---------------------------------------------------
  echo
  echo "## emitted rows, first $SWEEP top-15k queries x full PyPI"
  echo
  for mode_args in "--typosquat" "--typosquat --pypi" "-t 1"; do
    n=0
    while read -r q; do
      # shellcheck disable=SC2086
      c=$("$BIN" --string "$q" --list "$LIST" $mode_args 2>/dev/null | wc -l || true)
      n=$((n + c))
    done < <(head -"$SWEEP" "$TOP")
    printf "%-24s %d\n" "$mode_args" "$n"
  done
} > "$report"

if [ "$mode" = capture ]; then
  echo "wrote $report"
  cat "$report"
  exit 0
fi

if diff -u "$OUT/quality.txt" "$report"; then
  echo "OK: recall and emitted-row counts unchanged"
else
  echo "FAIL: quality gate moved — see the diff above" >&2
  echo "(if the change is intended, re-run: scripts/quality.sh capture)" >&2
  exit 1
fi
