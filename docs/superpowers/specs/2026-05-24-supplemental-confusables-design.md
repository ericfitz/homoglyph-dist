# Design: sqdist supplemental confusables (`--confusables=<list>`) (Phase 4)

Date: 2026-05-24
Status: Approved
Component: `sqdist` — adds opt-in non-UTS#39 confusable data (FlowCrypt single-char
supplement + 4 curated digraphs) behind a `--confusables=<list>` source selector,
by source-parameterizing the skeleton lookup. Builds on v0.3.0. **Not a new axis.**

## Summary

Today the skeleton axes (`skeleton_levenshtein`, `skeleton_damerau`,
`confusable_only`) reduce strings to UTS#39 skeletons via a single global
`CONFUSABLES` table. This phase makes the skeleton lookup **source-parameterized**:
an active `ConfusableMap` (single-char map + digraph list) is built from the
sources selected by a new `--confusables=<comma-list>` flag (default `uts39`) and
threaded through `PairContext`. Two supplemental sources are added — `flowcrypt`
(single-char, from FlowCrypt's MIT homograph DB, anchored to UTS#39) and `digraph`
(4 hand-curated multi-char mappings). When enabled, their entries enter the active
map and every existing skeleton axis benefits automatically.

**No new axis, metric, verdict logic, or `AxisValue` variant.** The only structural
change is parameterizing the skeleton lookup; everything else is data + a flag +
docs. Default behavior (`uts39` only) is byte-for-byte unchanged — preserving the
authoritative-data guarantee and keeping the high-FP `cl→d` out of the default
path. Folded into the unreleased **v0.3.0**.

## CLI: `--confusables=<comma-list>`

Default `uts39`. Recognized sources:
- `uts39` — the standard UTS#39 data (always implicitly on; including it explicitly
  is a no-op, omitting it does NOT disable it — UTS#39 is the authoritative base).
- `flowcrypt` — single-char supplement (FlowCrypt MIT DB, filtered + UTS#39-anchored).
- `digraph` — the 4 curated digraph→single-char mappings (`vv→w`, `cl→d`, `rn→m`,
  `nn→m`).

Parsed/validated like `--fields`/`--metric`: split on commas, trim, dedup; unknown
source → error naming the offender and listing valid sources (`uts39, flowcrypt,
digraph`), exit 2. An empty/whitespace value resolves to just `uts39`.

Chosen over a standalone `--flowcrypt` boolean (sources are homogeneous and this
scales) and over a single `--extra-confusables` toggle (users want `flowcrypt`
*without* the higher-FP `digraph`).

## The one real refactor: `ConfusableMap` (source-parameterized lookup)

A new module `src/confusables.rs` (the runtime confusable model; distinct from the
generated `confusables_data.rs` / `flowcrypt_data.rs` / `digraph_data.rs` tables).

```rust
/// The active confusable model for a run: single-char skeleton map (UTS#39 plus
/// any enabled single-char supplements) and the enabled multi-char digraph rules.
pub struct ConfusableMap {
    /// code point -> skeleton string, sorted by key for binary search. UTS#39
    /// entries take precedence on key collision (inserted last / supplements
    /// skipped when the key already exists).
    singles: Vec<(u32, &'static str)>,
    /// (digraph source, replacement), e.g. ("vv","w"). Empty unless `digraph`
    /// enabled. Longest-match-first during skeletonization.
    digraphs: &'static [(&'static str, &'static str)],
}

impl ConfusableMap {
    /// Build from the enabled source set. `uts39` is always included.
    pub fn from_sources(sources: &Sources) -> Self;

    /// Skeleton of one char (single-char map only). None if unmapped.
    pub fn skeleton_of(&self, c: char) -> Option<&str>;

    /// Are two chars confusable under this map's single-char skeletons?
    pub fn confusable(&self, a: char, b: char) -> bool;

    /// Full skeleton of a string: longest-match-first over digraph source keys,
    /// else per-char single-char mapping, else the char unchanged.
    pub fn skeleton(&self, s: &str) -> String;
}
```

`Sources` is a small parsed struct/flags (e.g. `{ flowcrypt: bool, digraph: bool }`;
`uts39` always on). `from_sources` builds `singles` by starting from UTS#39
(`confusables_data::CONFUSABLES`) and, if `flowcrypt` enabled, merging
`flowcrypt_data::FLOWCRYPT` entries **only for keys UTS#39 doesn't already define**
(UTS#39 wins); then sorts by key. `digraphs` is set to `digraph_data::DIGRAPHS`
when `digraph` enabled, else `&[]`.

### Skeletonization with digraphs (longest-match-first)

`skeleton(s)` walks the string by char index. At each position, if any digraph
source key matches the upcoming chars (try the digraphs; all current ones are
length 2), consume it and append the replacement; else map the single char via
`skeleton_of` (or pass it through). Greedy, longest-match-first (longest digraph
key first — currently all len 2, so order among them doesn't matter, but the
matcher is written longest-first to be correct if a 3-char digraph is ever added).
One-way only: `vv→w`, never `w→vv`.

> Idempotence note: UTS#39 skeletons are idempotent (`skeleton(skeleton(s)) ==
> skeleton(s)`). Digraphs can break strict idempotence (e.g. `nn→m` then `m→rn`
> under UTS#39 would give `nn→rn`). This is acceptable — `skeleton()` is a single
> non-recursive pass (digraph rewrite + single-char map applied once per position),
> matching the existing single-pass design; we do NOT iterate to a fixed point.
> Document that supplemental skeletons are single-pass.

### Threading through `PairContext`

`PairContext::new` gains a `&ConfusableMap` parameter; it calls `cmap.skeleton(a)`
/ `cmap.skeleton(b)` instead of the free `distance::skeleton`. `main` parses
`--confusables`, builds the `ConfusableMap` once, and passes `&cmap` into every
`PairContext::new` (single-pair, stdin batch, and each list candidate). The
`uts39_confusable_count` axis uses `cmap.confusable(...)` instead of
`distance::confusable(...)`. The `align` traceback (`distance::align`) is unchanged
(it operates on the raw char vecs, not skeletons).

### Disposition of the old free functions in `distance.rs`

`distance::skeleton_of` / `skeleton` / `confusable` currently read the global
`CONFUSABLES`. They are replaced by `ConfusableMap` methods. The free `skeleton_of`
helper logic (binary search over a sorted slice) moves into `ConfusableMap`
(generalized over `self.singles`). Remove the now-unused free functions to avoid
dead code (no `#[allow]`). `distance.rs` keeps the pure edit-distance algorithms
(`levenshtein`, `damerau`, `align`, `AlignOp`); the skeleton/confusable model moves
to `confusables.rs`. (This is a clean responsibility split — distances vs. the
confusable model — and the move is small.)

## Data / generators

### `digraph` — hand-written, no generator
`src/digraph_data.rs`:
```rust
//! Curated digraph -> single-char confusable mappings (non-UTS#39, opt-in via
//! --confusables=digraph). Seeded by dnstwist's glyphs_ascii (Apache-2.0); this
//! short curated list is our own. One-way (digraph impersonates the single char).
//!
//! FP note for future maintainers: cl->d is the highest-FP rule (clear->dear,
//! clock->dock). If FP complaints arise, the menu of mitigations (NOT built now,
//! the opt-in is the consent): same-script-Latin gating, a short-name/font-context
//! gate (rn->m is a small-size proportional-font effect), a per-pair deny list,
//! and tiny-edit-distance gating. See the research doc.
pub static DIGRAPHS: &[(&str, &str)] = &[("vv", "w"), ("cl", "d"), ("rn", "m"), ("nn", "m")];
```

### `flowcrypt` — generated from the FlowCrypt MIT DB
`scripts/gen_flowcrypt.py` (pure stdlib, like `gen_confusables.py`) → emits
`src/flowcrypt_data.rs`.

**Source (verified):** `homograph/homographs.json` in
`github.com/FlowCrypt/idn-homographs-database` (LICENSE = MIT; ~19.3 MB). Schema
(verified against the repo): a JSON object keyed by a base character string; each
value is `{ "codepoint", "lang", "name", "similar_char": [ { "char", "codepoint",
"lang", ... }, ... ] }`. A base char `B` maps to a list of chars `S` that look like
`B`.

**Generation algorithm:**
1. Load `homographs.json` and the UTS#39 `confusables.txt` (same file
   `gen_confusables.py` uses, for anchoring) — or reuse the already-generated
   single-char UTS#39 map.
2. **Filter to Latin-confusable:** keep only base chars `B` that are ASCII
   (`ord(B) < 128`; equivalently `lang == "Basic Latin"`). This drops the
   CJK/Hangul bulk and yields "characters that look like an ASCII char" — the
   typosquat-relevant subset.
3. **Anchor to UTS#39:** for each kept base `B` and each `similar_char` `S`, the
   target skeleton is `uts39_skeleton(B)` — the skeleton UTS#39 assigns to `B`
   (or `B` itself if UTS#39 leaves it unmapped). Emit `(ord(S) -> uts39_skeleton(B))`.
   This makes a FlowCrypt look-alike collapse to the same skeleton its ASCII
   partner already has under UTS#39.
4. **UTS#39 wins on conflict (at generation AND runtime):** if `S` is already a
   UTS#39 source code point, the runtime merge skips the FlowCrypt entry (UTS#39
   precedence in `from_sources`). The generator additionally LOGS any `S` where
   the FlowCrypt-derived skeleton disagrees with UTS#39's existing skeleton for `S`
   (to stderr, as a count + sample) so divergences are visible, but emits the row
   anyway (runtime precedence handles it).
5. **Dedup + sort:** collapse duplicate `S` keys (a look-alike pointing at several
   ASCII bases → keep the first/lowest-base deterministically; log multiplicity),
   sort by code point, emit `pub static FLOWCRYPT: &[(u32, &str)]` (same shape as
   `CONFUSABLES`).

Expected size: a few thousand rows (filtered from ~13K). The vendored 19.3 MB JSON
is NOT committed — only the generator and its derived `flowcrypt_data.rs` output
(the generator fetches/loads the JSON from a path or URL at generation time, like
the `confusables.txt` workflow). Document the regeneration command. Add a NOTICE/
attribution comment in `flowcrypt_data.rs` (FlowCrypt, MIT).

> Implementation note: the generator needs the JSON locally. The plan's data task
> fetches `homograph/homographs.json` from the repo (raw URL) to a temp/ignored
> path, runs the generator, and commits only `flowcrypt_data.rs`. If the fetch is
> unavailable in the build environment, the generator must accept a `--input
> <path>` so a manually-downloaded copy works.

## Data-source provenance in `-v`/`--version` (required)

**Every embedded external data source must carry its version+date provenance, and
`-v` must expose it.** Where a source has no clear version (or changes its data
without reversioning), track the upstream repo **commit id** the data was retrieved
from, plus the retrieval date.

- **UTS#39 confusables** (`confusables_data.rs`, pre-existing): has a clear version.
  `gen_confusables.py` emits `pub static CONFUSABLES_PROVENANCE: &str = "UTS#39
  confusables.txt v<UNICODE_VERSION> (<date>)";` (e.g. `v17.0.0 (2025-07-22)` — the
  values already in the file's header comment, now promoted to a constant).
- **FlowCrypt** (`flowcrypt_data.rs`, new): the `idn-homographs-database` repo has
  **no releases/tags** — data on `master` can change without reversioning — so track
  the **commit SHA** + retrieval date. `gen_flowcrypt.py` resolves and embeds
  `pub static FLOWCRYPT_PROVENANCE: &str = "FlowCrypt idn-homographs-database @
  <short-sha> (retrieved <date>)";`. The generator obtains the commit SHA from the
  GitHub API (`/repos/FlowCrypt/idn-homographs-database/commits/master`) or, when
  given a local `--input`, from a `--source-commit <sha>`/`--source-date <date>`
  argument the operator supplies.

**`-v` output format (chosen — multi-line):** line 1 unchanged, then one indented
`data:` line per embedded source, always printed (the tables are compiled in
regardless of `--confusables`):
```
sqdist 0.3.0 (d256784)
  data: UTS#39 confusables.txt v17.0.0 (2025-07-22)
  data: FlowCrypt idn-homographs-database @ a1b2c3d (retrieved 2026-05-24)
```
`main.rs`'s version arm prints `env!("CARGO_PKG_VERSION")` + `SQDIST_GIT_SHA` (line
1) then `confusables_data::CONFUSABLES_PROVENANCE` and
`flowcrypt_data::FLOWCRYPT_PROVENANCE` (the `data:` lines). The digraph source is
**hand-authored, not external data**, so it has no provenance line (its "version" is
the source tree itself; document this in the digraph_data.rs comment). This format
scales: each new external data table adds its own `*_PROVENANCE` constant and one
`data:` line.

This requirement applies whether or not FlowCrypt ships in this phase; since
FlowCrypt IS shipping here, both provenance constants land now. (Retrofitting the
UTS#39 provenance constant is a small change to `gen_confusables.py` +
`confusables_data.rs` + the `-v` arm.)

## File layout / change set

| File | Change |
|---|---|
| `src/confusables.rs` | NEW. `ConfusableMap`, `Sources`, `from_sources`, `skeleton_of`/`confusable`/`skeleton` methods (+ digraph longest-match), source parsing/validation. Unit tests. |
| `src/digraph_data.rs` | NEW. The 4-entry `DIGRAPHS` table + FP-mitigation comment. |
| `src/flowcrypt_data.rs` | NEW (generated). `pub static FLOWCRYPT: &[(u32,&str)]` + `pub static FLOWCRYPT_PROVENANCE: &str` (repo commit + retrieval date). |
| `scripts/gen_flowcrypt.py` | NEW. Stdlib generator (filter + anchor + dedup + conflict log); resolves the source commit SHA (GitHub API or `--source-commit`) and emits `FLOWCRYPT_PROVENANCE`. |
| `src/confusables_data.rs` | regenerate to add `pub static CONFUSABLES_PROVENANCE: &str = "UTS#39 confusables.txt v17.0.0 (2025-07-22)";` (promote the header-comment version to a constant). |
| `scripts/gen_confusables.py` | emit the `CONFUSABLES_PROVENANCE` constant from the Unicode version/date it already parses. |
| `src/distance.rs` | REMOVE `skeleton_of`/`skeleton`/`confusable` (moved to confusables.rs); keep `levenshtein`/`damerau`/`align`/`AlignOp`. |
| `src/axes.rs` | `PairContext::new(a, b, &ConfusableMap)`; build skeletons via the map; `Uts39ConfusableCount` uses `cmap.confusable`. (Note: the `uts39_confusable_count` axis still counts via the map's `confusable`, which now may include supplements when enabled — see "Naming note".) |
| `src/main.rs` | add `mod confusables; mod digraph_data; mod flowcrypt_data;`; parse `--confusables`; build the `ConfusableMap` once; thread `&cmap` into all `PairContext::new` calls; add a `--confusables` line to `--help`; extend the `-v`/`--version` arm to print the two `data:` provenance lines after line 1. |
| `README.md`, `CLAUDE.md` | document `--confusables`, the sources, the "less authoritative / opt-in" caveat, the regeneration command, and the new module/data files. |
| `Cargo.toml` | `include` list: add `scripts/gen_flowcrypt.py` (mirrors the existing `gen_confusables.py` include). No new dependency. Version stays 0.3.0. |

### Naming note (decided)

The axis is named `uts39_confusable_count`. When `--confusables` enables
supplements, `cmap.confusable()` reflects the active set, so the count could
include non-UTS#39 confusables — making the `uts39_` prefix slightly inaccurate
under a non-default flag. **Decision: keep the name `uts39_confusable_count`
unchanged** (renaming churns the output contract for an experimental axis, and the
default path IS pure UTS#39). Document that under a non-default `--confusables`,
this experimental axis counts confusables from the active set, not strictly UTS#39.
(`uts39_skeleton_delta` is likewise computed from the active-map skeletons.) This
is acceptable for experimental axes; revisit if either is promoted.

## FP guards: documented, NOT implemented (decided)

Digraph matching is a plain greedy longest-match-first rewrite — no special gating.
The `--confusables=...,digraph` opt-in is itself the user's consent to the FP
tradeoff (e.g. `cl→d` collapsing `clear`→`dear`). The research-derived mitigations
are recorded as comments on `digraph_data.rs` / the matcher (see the DIGRAPHS doc
comment above), NOT built — so a future maintainer seeing FP complaints has the
menu without speculative logic shipped now.

## Testing

`src/confusables.rs` unit tests:
- Default (`uts39` only): `skeleton`/`confusable` byte-identical to the old
  `distance::*` behavior on the existing test corpus (port the moved tests:
  `digit_letter_confusable`, `skeleton_maps_multichar` m↔rn, `skeleton_is_idempotent`,
  `skeleton_collapses_homoglyphs`, `vv_w_and_cl_d_are_not_uts39_confusables`).
- Source parsing: `uts39` default; `uts39,flowcrypt,digraph` all-on; unknown source
  → error listing valid names; dedup; empty → uts39.
- `digraph` enabled: `skeleton("vv") == skeleton("w")` and `skeleton("devflovv")
  == skeleton("devflow")` (the vv/w gap CLOSES under `digraph`); `cl→d`, `nn→m`
  likewise. With `digraph` DISABLED (default), the vv/w gap STAYS (regression
  pin from Phase 1 — `skeleton("vv") != skeleton("w")` by default).
- `flowcrypt` enabled: a FlowCrypt-only look-alike char collapses to its ASCII
  partner's skeleton (pick a concrete pair from the generated table); UTS#39
  precedence — a char defined by both resolves to the UTS#39 skeleton.
- Longest-match-first: a string where a digraph overlaps a single-char mapping
  resolves digraph-first.

`src/axes.rs` / integration tests:
- With default sources, the whole panel is unchanged vs current behavior (the
  existing axis tests must still pass — they now go through a default `ConfusableMap`).
- `confusable_only` becomes true for `devflovv`/`devflow` ONLY when `digraph`
  enabled; false by default.

`src/main.rs`:
- `--confusables` parsing via `parse_from` (default, valid combos, unknown → err).
- End-to-end: `--confusables=uts39,digraph` makes `devflovv`/`devflow` score
  `confusable_only:true` / `skeleton_damerau:0`; default does not.
- **Provenance:** `-v`/`--version` output contains both `data:` lines — a test
  asserting `CONFUSABLES_PROVENANCE` mentions `UTS#39`/`17.0.0` and
  `FLOWCRYPT_PROVENANCE` mentions `FlowCrypt` + a commit/date (both non-empty,
  exact format pinned). Confirm the constants compile-in regardless of
  `--confusables` (the tables are always embedded).

`scripts/gen_flowcrypt.py`: a small self-test or a documented manual check (the
generator is stdlib; at minimum, running it on the real input produces a
non-empty, sorted, deduped `flowcrypt_data.rs` that `cargo build` accepts and the
flowcrypt tests pass against).

Standing gates: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check` all clean. No `#[allow(dead_code)]` (the old free functions are
removed, not silenced; the new data tables are consumed when their source is built,
but the static tables are referenced by `from_sources` so they're live).

## Docs

- README/CLAUDE: `--confusables=<list>` with the three sources; the default-pure-
  UTS#39 guarantee; the "supplemental sources are less authoritative / opt-in"
  caveat (FlowCrypt MIT single-char; digraph curated, `cl→d` high-FP); the FlowCrypt
  regeneration command (`gen_flowcrypt.py`); the new files. Note the skeleton axes'
  behavior now depends on `--confusables`.
- A NOTICE/attribution for FlowCrypt (MIT) in `flowcrypt_data.rs` and the docs.

## Out of scope / deferred

- CJK pseudo-homoglyph (permanently deferred).
- Implemented FP guards (documented only).
- Renaming `uts39_*` axes (kept; documented caveat).
- Iterating skeletonization to a fixed point (single-pass, as today).
- A `--confusables` value affecting the verdict beyond what the skeleton axes
  already feed it (no verdict change).

## Risks / notes

- **FlowCrypt fetch dependency at generation time** — mitigated by `--input <path>`
  on the generator. The derived table is committed; the 19.3 MB JSON is not.
- **Filtering heuristic** (ASCII-base) may include or exclude edge entries; the
  conflict log + the "opt-in, less authoritative" framing bound the risk. Validate
  the generated table size and spot-check a few entries during implementation.
- **Idempotence** under digraphs is single-pass (documented), not a fixed point.
- **`distance.rs` → `confusables.rs` move** touches every skeleton call site; the
  default-behavior-unchanged tests are the safety net.
