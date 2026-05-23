#!/usr/bin/env bash
# Package the signed/notarized binary into a tarball, checksum it, and create
# a GitHub Release with the artifact attached. Reads the version from Cargo.toml.
#   ./release/package-release.sh            # tag = v<Cargo.toml version>
#   ./release/package-release.sh v0.2.0     # explicit tag
set -euo pipefail
# shellcheck source=release/config.sh
source "$(dirname "${BASH_SOURCE[0]}")/config.sh"

if [[ ! -f "$UNIVERSAL_BIN" ]]; then
    echo "error: $UNIVERSAL_BIN not found — run build.sh and sign-notarize.sh first" >&2
    exit 1
fi

VERSION="$(grep -m1 '^version' "${REPO_ROOT}/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')"
TAG="${1:-v${VERSION}}"
TARBALL="${DIST_DIR}/${BIN_NAME}-${TAG}-macos-universal.tar.gz"

echo "==> Packaging $TARBALL"
tar -C "$DIST_DIR" -czf "$TARBALL" "$BIN_NAME"

echo "==> Computing SHA-256 (needed for the Homebrew formula)"
SHA="$(shasum -a 256 "$TARBALL" | awk '{print $1}')"
echo "$SHA  $(basename "$TARBALL")" | tee "${TARBALL}.sha256"

echo "==> Creating GitHub Release $TAG"
gh release create "$TAG" \
    --repo "$GH_REPO" \
    --title "$TAG" \
    --generate-notes \
    "$TARBALL" "${TARBALL}.sha256"

cat <<EOF

==> Done. Update the Homebrew formula with:
      url    https://github.com/${GH_REPO}/releases/download/${TAG}/$(basename "$TARBALL")
      sha256 ${SHA}
    Then run ./release/update-formula.sh ${TAG} ${SHA}
EOF
