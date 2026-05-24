# Implementation kickoff — sqdist redesign

Paste the prompt below to start the implementation session. It points at the
committed specs (the source of truth) rather than restating them.

---

We're implementing the planned redesign of `sqdist` (the Rust CLI at
/Users/efitz/Projects/homoglyph-dist). All design work is done and committed as
specs under docs/superpowers/specs/ — read these first, in this order, they are
the source of truth:

1. 2026-05-23-multi-axis-architecture-design.md   (APPROVED — the foundational refactor)
2. 2026-05-23-new-axes-research.md                (research findings + per-axis/feature designs)

Also read CLAUDE.md and skim src/main.rs to ground yourself in the current
(v0.2.0, shipped) code.

Background you need:
- sqdist is shipped at v0.2.0 on three channels (crates.io, Homebrew tap
  ericfitz/homebrew-tap, GitHub Releases with signed/notarized .pkg). Release
  tooling lives in release/. Do NOT cut a release or publish unless I explicitly
  ask.
- Single owner, pre-1.0, breaking changes approved. Commit directly to main
  (my standing consent). Pushes use SSH and need a physical key touch — if a
  push fails on the key, stop and tell me, don't work around it.
- Per my global practice: run cargo test + cargo clippy --all-targets + cargo
  fmt --check before every commit; the tree is currently warning-clean, keep it
  that way. Conventional Commits; end commit messages with the
  Co-Authored-By: Claude Opus 4.7 (1M context) trailer.

Scope and sequence (each is its own brainstorm-if-needed → writing-plans →
subagent-driven implementation cycle; do them in order, stop for my review
between them):

  Phase 1 — Multi-axis architecture refactor (the approved spec). This is the
  foundation: Axis trait + ALL_AXES registry + PairContext, two-phase
  base/derived computation, the 8-axis panel, split main.rs into
  distance.rs/axes.rs/verdict.rs, drop the weighted hogl metric / normalized
  fields / --hogl-weight / Field enum, generalize --metric to any numeric axis.
  Breaking change → this becomes v0.3.0. The architecture spec is approved, so
  go straight to writing-plans for it.

  Phase 2 — script-count axis (highest-value new axis; UTS#39 restriction level,
  likely via the unicode-security crate — see research doc for the
  dependency-vs-DIY decision to make).

  Phase 3 — keyboard-distance axis (self-contained QWERTY coord table; note the
  research flagged this needs an AxisValue N/A representation for non-ASCII
  input — confirm that's handled in Phase 1 or add it here).

  Phase 4 — supplemental confusables feature (digraph + FlowCrypt, one feature
  behind --confusables=uts39,flowcrypt,digraph; requires source-parameterizing
  the skeleton lookup. The digraph source is exactly 4 curated one-way mappings
  vv→w/cl→d/rn→m/nn→m — see the "Unified supplemental confusables" section).

  CJK pseudo-homoglyph is explicitly DEFERRED — do not implement.

Start with Phase 1: read the two specs, then invoke the writing-plans skill to
produce the implementation plan for the architecture refactor, and we'll execute
it with subagent-driven development as we did for v0.1.0 and v0.2.0. Confirm your
understanding of the phased scope before you begin planning.
