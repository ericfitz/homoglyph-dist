# Release tooling

Scripts to build, sign, notarize, and publish `sqdist` for macOS. All outputs
land in `dist/` (gitignored). Edit [config.sh](config.sh) for identities/paths.

## One-time setup

1. **Notary keychain profile** — store your App Store credentials once:

   ```sh
   xcrun notarytool store-credentials sqdist-notary \
     --apple-id "<your-apple-id-email>" \
     --team-id 796T45968D \
     --password "<app-specific-password>"
   ```

   Generate the app-specific password at <https://account.apple.com> → Sign-In
   and Security → App-Specific Passwords. `sqdist-notary` must match
   `NOTARY_PROFILE` in config.sh.

2. **Homebrew tap repo** — create a GitHub repo named `homebrew-tap` (the
   `homebrew-` prefix is required). Users then `brew tap ericfitz/tap` and
   `brew install sqdist`.

## Cutting a release

```sh
./release/build.sh                    # universal arm64+x86_64 -> dist/sqdist
./release/sign-notarize.sh            # Developer ID sign + Apple notarization
./release/package-release.sh          # tarball + sha256 + GitHub Release (tag from Cargo.toml)
# copy the printed sha256, then:
TAP_DIR=~/path/to/homebrew-tap ./release/update-formula.sh v0.1.0 <sha256>
cd ~/path/to/homebrew-tap && git add -A && git commit -m "sqdist 0.1.0" && git push
```

Bump the version in `Cargo.toml` before building a new release; the tag defaults
to `v<that version>`.

## Why these steps

- **Universal binary** (`lipo`) runs on both Apple Silicon and Intel Macs.
- **Hardened runtime** (`--options runtime`) and a **secure timestamp**
  (`--timestamp`) are prerequisites for notarization.
- **Notarization** is what lets a *downloaded* binary open without a Gatekeeper
  block. A bare CLI binary can't be stapled (only app bundles / dmg / pkg can
  carry a stapled ticket), so we rely on the online notarization check; once
  Homebrew installs the binary from a notarized release, Gatekeeper accepts it.

## The three distribution channels

| Channel | Audience | Signing needed |
|---|---|---|
| **Homebrew tap** (these scripts) | any Mac user, `brew install` | yes — prebuilt binary |
| **GitHub Releases** (these scripts) | direct download | yes — prebuilt binary |
| **crates.io** (`cargo publish`) | Rust users, compiles locally | no — Gatekeeper never involved |

`cargo publish` is independent of this tooling: it ships source, so users build
locally and signing is irrelevant. Run it whenever you want a crates.io release.
