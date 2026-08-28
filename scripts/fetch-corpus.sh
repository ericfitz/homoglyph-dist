#!/usr/bin/env bash
# Download the evaluation corpora into a git-ignored corpus/ directory.
# Each output is newline-delimited names, which is what `sqdist --list` eats.
# Nothing here is committed: ~160 MB, changes upstream, re-fetches in under a minute.
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p corpus
cd corpus

fetch() {  # fetch <url> <outfile> [header]
  [ -s "$2" ] && { echo "  have $2"; return; }
  echo "  GET $2"
  curl -sSL --fail ${3:+-H "$3"} "$1" -o "$2.tmp" && mv "$2.tmp" "$2"
}

echo "PyPI names"
fetch https://pypi.org/simple/ pypi-simple.json 'Accept: application/vnd.pypi.simple.v1+json'
jq -r '.projects[].name' pypi-simple.json > pypi-names.txt

echo "npm names"
fetch https://unpkg.com/all-the-package-names/names.json npm-all.json
jq -r '.[]' npm-all.json > npm-names.txt

echo "PyPI top 15k"
fetch https://raw.githubusercontent.com/hugovk/top-pypi-packages/main/top-pypi-packages.json top-pypi.json
jq -r '(.rows // .)[].project' top-pypi.json > pypi-top15k.txt

echo "Labeled typosquat pairs"
fetch https://raw.githubusercontent.com/ecosyste-ms/typosquatting-dataset/main/typosquats.csv typosquats.csv

echo "OSV malicious names"
for eco in PyPI npm; do
  lc=$(echo "$eco" | tr '[:upper:]' '[:lower:]')
  fetch "https://osv-vulnerabilities.storage.googleapis.com/$eco/all.zip" "osv-$lc.zip"
  [ -s "osv-$lc-malicious.txt" ] && { echo "  have osv-$lc-malicious.txt"; continue; }
  d=$(mktemp -d); trap 'rm -rf "$d"' EXIT
  unzip -qo "osv-$lc.zip" 'MAL-*' -d "$d"
  # ponytail: one jq per file batch; fine at ~230k files, revisit if it gets slow
  find "$d" -name 'MAL-*.json' -print0 | xargs -0 jq -r '.affected[]?.package.name' \
    | sort -u > "osv-$lc-malicious.txt"
  rm -rf "$d"; trap - EXIT
done

echo
wc -l ./*.txt
