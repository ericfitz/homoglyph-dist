# Supplemental Confusables (`--confusables`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Source-parameterize sqdist's skeleton lookup behind `--confusables=uts39,flowcrypt,digraph` (default `uts39`), adding an opt-in FlowCrypt single-char supplement and 4 curated digraph mappings that feed the existing skeleton axes. Not a new axis.

**Architecture:** A new `src/confusables.rs` holds `ConfusableMap` (active single-char map + digraph list) built from a parsed `Sources` selection; its `skeleton`/`confusable`/`skeleton_of` methods replace the global-table free functions in `distance.rs`. `PairContext::new` gains a `&ConfusableMap` param; `main` builds the map once from `--confusables` and threads it everywhere. Digraph data is a hand-written 4-entry table; FlowCrypt data is generated (filter ASCII-base look-alikes → anchor to UTS#39 → dedup) into `src/flowcrypt_data.rs` by a new stdlib `scripts/gen_flowcrypt.py`.

**Tech Stack:** Rust 2021, std-only (no new dependency). Python 3 stdlib for the generator. `cargo test` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`.

---

## Context for the implementer (read before starting)

You are adding the supplemental-confusables feature to `sqdist`, a Rust CLI scoring string pairs for typosquat/homoglyph detection. **Read first:**
- `docs/superpowers/specs/2026-05-24-supplemental-confusables-design.md` — the approved spec (source of truth).
- `CLAUDE.md` — conventions.
- `src/distance.rs` — currently holds the skeleton model (`skeleton_of`/`skeleton`/`confusable`, reading the global `confusables_data::CONFUSABLES`) AND the edit-distance algorithms (`levenshtein`/`damerau`/`align`/`AlignOp`). This task MOVES the skeleton model out to a new `confusables.rs`.
- `src/axes.rs` — `PairContext::new(a, b)` builds skeletons via `distance::skeleton`; `Uts39ConfusableCount::compute` uses `distance::confusable`.
- `src/main.rs` — `fn score_pair(a, b) -> Panel` wraps `build_panel(&PairContext::new(a, b))`; it's the chokepoint, called from many tests. `parse_from(Vec<String>) -> Result<Opts, String>` parses CLI; `Opts` holds parsed flags.

**The model being introduced (`ConfusableMap`, in `src/confusables.rs`):**
```rust
pub struct Sources { pub flowcrypt: bool, pub digraph: bool } // uts39 always on
pub struct ConfusableMap {
    singles: Vec<(u32, &'static str)>,             // sorted by key; UTS#39 wins on collision
    digraphs: &'static [(&'static str, &'static str)],
}
impl ConfusableMap {
    pub fn from_sources(sources: &Sources) -> Self;
    pub fn uts39() -> Self;                          // convenience = from_sources(&Sources::default())
    pub fn skeleton_of(&self, c: char) -> Option<&str>;
    pub fn confusable(&self, a: char, b: char) -> bool;
    pub fn skeleton(&self, s: &str) -> String;       // longest-match-first digraphs, then single-char
}
```
`Sources::default()` = `{flowcrypt:false, digraph:false}` (pure UTS#39). `parse_sources(&str) -> Result<Sources, String>` parses the `--confusables` comma-list.

**Working rules (CLAUDE.md):**
- TDD; before EVERY commit run `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — all clean.
- Conventional Commits; end every message with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.
- Commit directly to `main`. Do NOT push/release. Do NOT bump version (stays 0.3.0). No `#[allow(dead_code)]` (the moved functions are removed, not silenced).
- **Default behavior (`uts39` only) must be byte-for-byte unchanged.** Task 1's ported tests are the safety net.

**Key threading fact:** `PairContext::new` is called at: `src/main.rs:26` (in `score_pair`), `src/verdict.rs:103` (test helper `panel`), `src/axes.rs:480` (test helper `run`) and `:607` (a test). `score_pair` is called from ~13 sites (mostly tests). Strategy: add the `&ConfusableMap` param to `PairContext::new` and `score_pair`; update the THREE test helpers (`run` in axes.rs, `panel` in verdict.rs, `score_pair` in main.rs) to construct a default map internally so individual test call sites stay unchanged where possible. See Task 1 for the exact helper shapes.

---

## Task 0: Retrofit UTS#39 data-source provenance into `-v`

Independent, pre-existing-data change (do first — it's small and unrelated to the refactor). Promote the UTS#39 version/date (already in `confusables_data.rs`'s header comment) to a `CONFUSABLES_PROVENANCE` constant the generator emits, and print it in `-v`. (The FlowCrypt provenance line is added in Tasks 3–4; this task wires the framework + the first source.)

**Files:**
- Modify: `scripts/gen_confusables.py` (emit the constant), `src/confusables_data.rs` (add the constant — hand-add now to match what the generator will emit; the file is generated but we're not re-downloading Unicode data this task), `src/main.rs` (`-v` arm + a test)

- [ ] **Step 1: Add `CONFUSABLES_PROVENANCE` to `src/confusables_data.rs`**

The file header comment already reads `// Auto-generated from Unicode UTS#39 confusables.txt (v17.0.0, 2025-07-22).`. Add this constant immediately after the header comment block and before `pub static CONFUSABLES`:
```rust
/// Provenance of the embedded UTS#39 confusables data, surfaced by `-v`.
pub static CONFUSABLES_PROVENANCE: &str = "UTS#39 confusables.txt v17.0.0 (2025-07-22)";
```

- [ ] **Step 2: Make `gen_confusables.py` emit the constant on regeneration**

So a future regeneration keeps the constant in sync. Read `scripts/gen_confusables.py`; it parses the Unicode version + date from `confusables.txt` (the header line like `# confusables.txt ... Version: 17.0.0 ... Date: 2025-07-22`) or already prints them in the output header comment. Add code so the generated output includes, right after the header comment lines and before the `pub static CONFUSABLES` line:
```python
print(f'/// Provenance of the embedded UTS#39 confusables data, surfaced by `-v`.')
print(f'pub static CONFUSABLES_PROVENANCE: &str = "UTS#39 confusables.txt v{version} ({date})";')
print()
```
(Use whatever variables the script already holds for the parsed version/date — if it currently only embeds them in the comment string, reuse those same values for the constant. If the script does not currently parse them into variables, parse them from the same header line it uses for the comment. Do NOT re-download Unicode data; this step only changes what the generator WOULD emit — verified by reading the script, not by running it against the network.)

- [ ] **Step 3: Print provenance in `-v` + failing test**

Add to `src/main.rs` `tests`:
```rust
    #[test]
    fn version_includes_confusables_provenance() {
        // The provenance constant is non-empty and names the source + version.
        let p = confusables_data::CONFUSABLES_PROVENANCE;
        assert!(p.contains("UTS#39"), "provenance names the standard: {p}");
        assert!(p.contains("17.0.0"), "provenance names the version: {p}");
    }
```
Run: `cargo test version_includes_confusables_provenance 2>&1 | head` → FAIL (constant not referenced / not present until Step 1 compiled; if Step 1 done it may pass — that's fine, this test PINS it). Then update the `-v`/`--version` arm in `parse_from` to print the data line. Current arm:
```rust
            "-v" | "--version" => {
                println!(
                    "sqdist {} ({})",
                    env!("CARGO_PKG_VERSION"),
                    env!("SQDIST_GIT_SHA")
                );
                std::process::exit(0);
            }
```
Change to:
```rust
            "-v" | "--version" => {
                println!(
                    "sqdist {} ({})",
                    env!("CARGO_PKG_VERSION"),
                    env!("SQDIST_GIT_SHA")
                );
                println!("  data: {}", confusables_data::CONFUSABLES_PROVENANCE);
                std::process::exit(0);
            }
```
(The FlowCrypt `data:` line is added in Task 4, after `flowcrypt_data` exists.)

- [ ] **Step 4: Run + gates + smoke**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` → all pass.
Run: `cargo build && ./target/debug/sqdist -v` → prints two lines:
```
sqdist 0.3.0 (<sha>)
  data: UTS#39 confusables.txt v17.0.0 (2025-07-22)
```

- [ ] **Step 5: Commit**

```bash
git add scripts/gen_confusables.py src/confusables_data.rs src/main.rs
git commit -m "$(cat <<'EOF'
feat: expose UTS#39 confusables data provenance in -v

Promote the embedded confusables version/date to a CONFUSABLES_PROVENANCE
constant (generator emits it) and print it as a `data:` line under -v. First
of the per-source provenance lines; FlowCrypt's follows when that table lands.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 1: Introduce `ConfusableMap`, move the skeleton model, thread a default map (behavior unchanged)

The big refactor. Create `confusables.rs` with `ConfusableMap` (UTS#39-only for now; no supplements yet), move `skeleton_of`/`skeleton`/`confusable` there as methods, thread a default map through `PairContext`/`score_pair`/test helpers, and delete the free functions. The whole existing test suite must still pass (default = UTS#39 = old behavior).

**Files:**
- Create: `src/confusables.rs`
- Modify: `src/distance.rs` (remove skeleton model + its 5 tests), `src/axes.rs` (`PairContext::new` param + call sites + `run` helper), `src/verdict.rs` (`panel` helper), `src/main.rs` (`mod confusables;`, `score_pair` param + helper)

- [ ] **Step 1: Create `src/confusables.rs` with `Sources`, `ConfusableMap` (UTS#39-only), and the moved tests**

```rust
//! The runtime confusable model: the active single-char skeleton map and digraph
//! rules for a run, selected by `--confusables`. Distinct from the generated
//! `confusables_data` (UTS#39), `flowcrypt_data`, and `digraph_data` tables.

use crate::confusables_data::CONFUSABLES;

/// Which supplemental confusable sources are enabled. UTS#39 is always on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub flowcrypt: bool,
    pub digraph: bool,
}

/// Parse a `--confusables` comma-list into `Sources`. `uts39` is always on
/// (listing it is a no-op; omitting it does not disable it). Unknown source →
/// Err naming the offender + valid names. Empty/whitespace → default (uts39).
pub fn parse_sources(spec: &str) -> Result<Sources, String> {
    let mut s = Sources::default();
    for raw in spec.split(',') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        match name {
            "uts39" => {}
            "flowcrypt" => s.flowcrypt = true,
            "digraph" => s.digraph = true,
            other => {
                return Err(format!(
                    "unknown confusable source: {other} (valid: uts39, flowcrypt, digraph)"
                ));
            }
        }
    }
    Ok(s)
}

/// The active confusable model for a run.
pub struct ConfusableMap {
    /// code point -> skeleton string, sorted by key. UTS#39 entries take
    /// precedence on key collision.
    singles: Vec<(u32, &'static str)>,
    /// (digraph source, replacement); longest-match-first. Empty unless enabled.
    digraphs: &'static [(&'static str, &'static str)],
}

impl ConfusableMap {
    /// UTS#39-only map (the default).
    pub fn uts39() -> Self {
        Self::from_sources(&Sources::default())
    }

    /// Build from the enabled source set. UTS#39 is always the base; enabled
    /// supplements are merged in only for keys UTS#39 does not already define
    /// (UTS#39 wins on collision). (flowcrypt/digraph wired in later tasks; for
    /// now this builds the UTS#39-only map regardless of flags.)
    pub fn from_sources(_sources: &Sources) -> Self {
        let singles: Vec<(u32, &'static str)> = CONFUSABLES.to_vec();
        // CONFUSABLES is already sorted by key; keep it sorted.
        ConfusableMap {
            singles,
            digraphs: &[],
        }
    }

    /// Skeleton of one char via the single-char map. None if unmapped.
    pub fn skeleton_of(&self, c: char) -> Option<&str> {
        let cp = c as u32;
        self.singles
            .binary_search_by(|&(k, _)| k.cmp(&cp))
            .ok()
            .map(|i| self.singles[i].1)
    }

    /// Are two chars confusable under this map's single-char skeletons?
    pub fn confusable(&self, a: char, b: char) -> bool {
        if a == b {
            return true;
        }
        let mut sa_buf = [0u8; 4];
        let mut sb_buf = [0u8; 4];
        let sa = self.skeleton_of(a).unwrap_or_else(|| a.encode_utf8(&mut sa_buf));
        let sb = self.skeleton_of(b).unwrap_or_else(|| b.encode_utf8(&mut sb_buf));
        sa == sb
    }

    /// Full skeleton of a string: longest-match-first over digraph source keys,
    /// else per-char single-char mapping, else the char unchanged. Single,
    /// non-recursive pass.
    pub fn skeleton(&self, s: &str) -> String {
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::with_capacity(s.len());
        let mut buf = [0u8; 4];
        let mut i = 0;
        while i < chars.len() {
            // Try digraphs longest-first. Current keys are all length 2; the
            // matcher is written to try longer keys first for future-proofing.
            let mut matched = false;
            // Find the longest digraph key that matches at position i.
            let mut best_len = 0usize;
            let mut best_rep = "";
            for &(src, rep) in self.digraphs {
                let klen = src.chars().count();
                if klen > best_len && i + klen <= chars.len() {
                    let window: String = chars[i..i + klen].iter().collect();
                    if window == src {
                        best_len = klen;
                        best_rep = rep;
                    }
                }
            }
            if best_len > 0 {
                out.push_str(best_rep);
                i += best_len;
                matched = true;
            }
            if !matched {
                let c = chars[i];
                match self.skeleton_of(c) {
                    Some(sk) => out.push_str(sk),
                    None => out.push_str(c.encode_utf8(&mut buf)),
                }
                i += 1;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> ConfusableMap {
        ConfusableMap::uts39()
    }

    #[test]
    fn digit_letter_confusable() {
        let m = m();
        assert!(m.confusable('1', 'l'));
        assert!(m.confusable('0', 'O'));
        assert!(!m.confusable('0', 'o'));
        assert!(m.confusable('I', 'l'));
        assert!(!m.confusable('x', 'y'));
    }

    #[test]
    fn skeleton_maps_multichar() {
        let m = m();
        assert_eq!(m.skeleton("microsoft"), m.skeleton("rnicrosoft"));
        assert_eq!(m.skeleton("microsoft"), "rnicrosoft");
    }

    #[test]
    fn skeleton_is_idempotent_for_uts39() {
        let m = m();
        for s in ["microsoft", "paypal", "vvallet", "g\u{43E}\u{43E}gle", "abc123"] {
            assert_eq!(m.skeleton(&m.skeleton(s)), m.skeleton(s), "not idempotent for {s}");
        }
    }

    #[test]
    fn skeleton_collapses_homoglyphs() {
        let m = m();
        assert_eq!(m.skeleton("p\u{0430}ypal"), m.skeleton("paypal"));
        assert_eq!(m.skeleton("xyz"), "xyz");
        assert_eq!(m.skeleton(""), "");
    }

    #[test]
    fn vv_w_and_cl_d_are_not_uts39_confusables_by_default() {
        // Default (uts39 only): the vv/w, cl/d gaps STAY (digraph source off).
        let m = m();
        assert_ne!(m.skeleton("vv"), m.skeleton("w"));
        assert_ne!(m.skeleton("cl"), m.skeleton("d"));
        assert_eq!(m.skeleton("m"), "rn"); // m->rn IS uts39, for contrast.
    }

    #[test]
    fn parse_sources_default_and_all() {
        assert_eq!(parse_sources("uts39").unwrap(), Sources::default());
        assert_eq!(parse_sources("").unwrap(), Sources::default());
        let all = parse_sources("uts39,flowcrypt,digraph").unwrap();
        assert!(all.flowcrypt && all.digraph);
        // dedup / order-insensitive
        assert_eq!(parse_sources("digraph,digraph").unwrap(), Sources { flowcrypt: false, digraph: true });
    }

    #[test]
    fn parse_sources_rejects_unknown() {
        let e = parse_sources("uts39,bogus").unwrap_err();
        assert!(e.contains("bogus"), "names offender: {e}");
        assert!(e.contains("flowcrypt"), "lists valid: {e}");
    }
}
```

- [ ] **Step 2: Remove the skeleton model + its 5 tests from `src/distance.rs`**

In `src/distance.rs`: delete `skeleton_of`, `skeleton`, `confusable` (now in confusables.rs), delete the `use crate::confusables_data::CONFUSABLES;` line, and update the module doc comment first line to `//! Edit-distance algorithms: Levenshtein and Damerau (OSA) + the alignment traceback.` Delete these 5 tests from the `distance.rs` `tests` module (moved to confusables.rs): `digit_letter_confusable`, `skeleton_maps_multichar`, `skeleton_is_idempotent`, `skeleton_collapses_homoglyphs`, `vv_w_and_cl_d_are_not_uts39_confusables`. Keep `levenshtein`/`damerau`/`align`/`AlignOp` and their tests. (Some distance tests use the `cv` helper — leave it; the align tests still use it.)

- [ ] **Step 3: Wire `mod confusables;` and thread the map through `PairContext`**

In `src/main.rs` add `mod confusables;` with the other module decls (sorted: after `mod confusables_data;`).

In `src/axes.rs`, change `PairContext::new` to take the map and use it:
```rust
    pub fn new(a: &'a str, b: &'a str, cmap: &crate::confusables::ConfusableMap) -> Self {
        let ca: Vec<char> = a.chars().collect();
        let cb: Vec<char> = b.chars().collect();
        let ska = cmap.skeleton(a);
        let skb = cmap.skeleton(b);
        let sva: Vec<char> = ska.chars().collect();
        let svb: Vec<char> = skb.chars().collect();
        let align = distance::align(&ca, &cb);
        PairContext { a, b, ca, cb, ska, skb, sva, svb, align }
    }
```
The `Uts39ConfusableCount` axis currently calls `distance::confusable(ctx.ca[i], ctx.cb[j])` and must use the active map instead. Give the axis access to the map by storing it on `PairContext`: add a `cmap: &'a ConfusableMap` field (the struct already has lifetime `'a`), set it in `new`, and have `Uts39ConfusableCount::compute` read `ctx.cmap.confusable(...)`. Updated struct:
```rust
pub struct PairContext<'a> {
    pub a: &'a str,
    pub b: &'a str,
    pub ca: Vec<char>,
    pub cb: Vec<char>,
    pub ska: String,
    pub skb: String,
    pub sva: Vec<char>,
    pub svb: Vec<char>,
    pub align: Vec<AlignOp>,
    pub cmap: &'a crate::confusables::ConfusableMap,
}
```
and set `cmap` in `new` (add the param `cmap` to the struct literal). Update `Uts39ConfusableCount::compute`:
```rust
            .filter(|&(i, j)| ctx.cmap.confusable(ctx.ca[i], ctx.cb[j]))
```

- [ ] **Step 4: Update the three test helpers + the two non-test call sites**

`src/main.rs` `score_pair` (the chokepoint) gains the map param:
```rust
fn score_pair(a: &str, b: &str, cmap: &confusables::ConfusableMap) -> Panel {
    build_panel(&PairContext::new(a, b, cmap))
}
```
Update `main`'s real call sites of `score_pair` (NON-test: lines ~103, ~376, ~399) to pass the run's map (which Task 4 builds from `--confusables`; for THIS task, since `--confusables` isn't parsed yet, pass `&confusables::ConfusableMap::uts39()` at each — Task 4 replaces these with the parsed map). Also update the single-pair/`PairContext` usage in `main` accordingly.

For the MANY `score_pair(...)` TEST call sites in `src/main.rs` tests: rather than edit each, add a test helper at the top of the `tests` module and rewrite calls to use it — OR give each call a default map. Cleanest: in the `main.rs` `tests` module add:
```rust
    fn cmap() -> confusables::ConfusableMap {
        confusables::ConfusableMap::uts39()
    }
```
and change each `score_pair("x", "y")` to `score_pair("x", "y", &cmap())`. (Do this for every `score_pair(` in the tests module — there are ~11.)

`src/axes.rs` test helper `run`:
```rust
    fn run(a: &str, b: &str) -> Panel {
        let cmap = crate::confusables::ConfusableMap::uts39();
        build_panel(&PairContext::new(a, b, &cmap))
    }
```
And the other `PairContext::new("paypal", ...)` test at axes.rs:607 — give it a local `let cmap = ConfusableMap::uts39();` and pass `&cmap`.

`src/verdict.rs` test helper `panel`:
```rust
    fn panel(a: &str, b: &str) -> Panel {
        let cmap = crate::confusables::ConfusableMap::uts39();
        build_panel(&PairContext::new(a, b, &cmap))
    }
```

- [ ] **Step 5: Run the full suite + gates**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: ALL existing tests pass (behavior unchanged — default map == UTS#39) PLUS the new confusables.rs tests. No clippy warnings (the free distance functions are gone, not orphaned). fmt clean. If any prior test fails, the refactor changed behavior — investigate; the default map must reproduce the old skeleton/confusable results exactly.

- [ ] **Step 6: Commit**

```bash
git add src/confusables.rs src/distance.rs src/axes.rs src/verdict.rs src/main.rs
git commit -m "$(cat <<'EOF'
refactor: source-parameterize the skeleton lookup via ConfusableMap

Move skeleton_of/skeleton/confusable out of distance.rs into a new
src/confusables.rs as ConfusableMap methods (UTS#39-only for now), add a
Sources struct + parse_sources, and thread a &ConfusableMap through
PairContext::new / score_pair / the test helpers. Default map == UTS#39, so
behavior is unchanged; supplemental sources (flowcrypt, digraph) land next.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add the `digraph` source (data + wiring)

Add the 4-entry digraph table and make `from_sources` include it when `digraph` is enabled. Now `--confusables=...,digraph` (once the CLI lands) closes the vv/cl/rn/nn gaps.

**Files:**
- Create: `src/digraph_data.rs`
- Modify: `src/main.rs` (`mod digraph_data;`), `src/confusables.rs` (`from_sources` uses it + tests)

- [ ] **Step 1: Create `src/digraph_data.rs`**

```rust
//! Curated digraph -> single-char confusable mappings (non-UTS#39, opt-in via
//! `--confusables=digraph`). Seeded by dnstwist's glyphs_ascii (Apache-2.0); this
//! short curated list is our own. One-way: the digraph impersonates the single char.
//!
//! FP note for future maintainers (mitigations NOT built — the opt-in is the
//! consent): cl->d is the highest-FP rule (clear->dear, clock->dock). If FP
//! complaints arise, the menu is: same-script-Latin gating; a short-name /
//! font-context gate (rn/m is a small-size proportional-font merging effect); a
//! per-pair deny list; tiny-edit-distance gating. See the research/design docs.
//!
//! ANCHORED TO UTS#39: each replacement is the UTS#39 skeleton of the impersonated
//! char, so the digraph unifies with it. UTS#39 maps m -> "rn", so "looks-like-m"
//! digraphs map to "rn": nn -> "rn". rn -> m is OMITTED — UTS#39 already unifies
//! rn<->m via m->rn; the inverse digraph would break rnicrosoft/microsoft. w/d are
//! unmapped in UTS#39, so vv -> "w" / cl -> "d" unify directly.
pub static DIGRAPHS: &[(&str, &str)] = &[("vv", "w"), ("cl", "d"), ("nn", "rn")];
```

- [ ] **Step 2: Add the failing test in `src/confusables.rs`**

Add to the `tests` module:
```rust
    #[test]
    fn digraph_source_closes_gaps_when_enabled() {
        let m = ConfusableMap::from_sources(&Sources { flowcrypt: false, digraph: true });
        // vv->w, cl->d close; nn anchors to m's UTS#39 skeleton "rn" so nn==m.
        assert_eq!(m.skeleton("vv"), m.skeleton("w"));
        assert_eq!(m.skeleton("devflovv"), m.skeleton("devflow"));
        assert_eq!(m.skeleton("cl"), m.skeleton("d"));
        assert_eq!(m.skeleton("nn"), m.skeleton("m")); // both -> "rn"
        assert_eq!(m.skeleton("rn"), m.skeleton("m")); // UTS#39 m->rn handles rn<->m
        // Default (digraph off) still has the gap (regression).
        let d = ConfusableMap::uts39();
        assert_ne!(d.skeleton("vv"), d.skeleton("w"));
    }

    #[test]
    fn digraph_does_not_break_uts39_rn_m() {
        // rn<->m must STILL unify with digraph enabled (we omit the inverting
        // rn->m entry precisely to preserve UTS#39's m->rn).
        let m = ConfusableMap::from_sources(&Sources { flowcrypt: false, digraph: true });
        assert_eq!(m.skeleton("rnicrosoft"), m.skeleton("microsoft"));
    }

    #[test]
    fn digraph_longest_match_first() {
        // A digraph match is taken before the per-char mapping at that position.
        let m = ConfusableMap::from_sources(&Sources { flowcrypt: false, digraph: true });
        // "cl" -> "d": the whole digraph collapses, not c then l.
        assert_eq!(m.skeleton("cl"), "d");
        // mid-word: "vvallet" -> "wallet"-skeleton.
        assert_eq!(m.skeleton("vvallet"), m.skeleton("wallet"));
    }
```
Run: `cargo test --lib digraph 2>&1 | head -20` → FAIL (digraphs empty in from_sources, gaps don't close).

- [ ] **Step 3: Wire `digraph` into `from_sources`**

In `src/main.rs` add `mod digraph_data;` (sorted). In `src/confusables.rs`, update `from_sources` to set `digraphs` when enabled:
```rust
    pub fn from_sources(sources: &Sources) -> Self {
        let singles: Vec<(u32, &'static str)> = CONFUSABLES.to_vec();
        let digraphs: &'static [(&'static str, &'static str)] = if sources.digraph {
            crate::digraph_data::DIGRAPHS
        } else {
            &[]
        };
        ConfusableMap { singles, digraphs }
    }
```
(Remove the `_sources` underscore now that it's used.)

- [ ] **Step 4: Run + gates**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: the 2 new digraph tests pass; the default-behavior tests (vv/w gap stays by default) still pass; all else green.

- [ ] **Step 5: Commit**

```bash
git add src/digraph_data.rs src/confusables.rs src/main.rs
git commit -m "$(cat <<'EOF'
feat(confusables): add the digraph source (vv/cl/rn/nn)

A 4-entry one-way digraph->single-char table, enabled via the digraph source.
from_sources includes it when selected; skeleton() matches it longest-first.
Closes the vv/w, cl/d, nn/m gaps only under --confusables=digraph; default
(uts39) behavior unchanged. FP mitigations documented as comments, not built.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: FlowCrypt generator + data + source

Write `scripts/gen_flowcrypt.py` (fetch/load the FlowCrypt MIT DB, filter ASCII-base look-alikes, anchor to UTS#39, dedup, emit `src/flowcrypt_data.rs`), run it to produce the table, and wire the `flowcrypt` source into `from_sources`.

**Files:**
- Create: `scripts/gen_flowcrypt.py`, `src/flowcrypt_data.rs` (generated)
- Modify: `src/main.rs` (`mod flowcrypt_data;`), `src/confusables.rs` (`from_sources` merges flowcrypt with UTS#39 precedence + tests), `Cargo.toml` (include the new script)

- [ ] **Step 1: Write `scripts/gen_flowcrypt.py`**

A pure-stdlib generator. It reads the FlowCrypt `homographs.json` and the UTS#39 `confusables.txt` (for anchoring), and emits `src/flowcrypt_data.rs`. Verified inputs: FlowCrypt DB is `homograph/homographs.json` in `github.com/FlowCrypt/idn-homographs-database` (MIT); schema is `{ base_char: { "lang", "similar_char": [ { "char", ... }, ... ] }, ... }`.

Create `scripts/gen_flowcrypt.py`:
```python
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
    lines.append(
        f'pub static FLOWCRYPT_PROVENANCE: &str = '
        f'"FlowCrypt idn-homographs-database @ {commit} (retrieved {source_date})";'
    )
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
```

- [ ] **Step 2: Run the generator to produce `src/flowcrypt_data.rs`**

Run (tries the network defaults; if the environment can't fetch, download the two files manually and pass `--homographs <path> --confusables <path>`):
```bash
python3 scripts/gen_flowcrypt.py
```
Expected: writes `src/flowcrypt_data.rs` with a `FLOWCRYPT_PROVENANCE` constant AND `pub static FLOWCRYPT: &[(u32, &str)] = &[ ... ];`, a few thousand entries, sorted by code point. The stderr line reports the count + the resolved commit + date + any disagreements/dups. Sanity-check the file:
```bash
head -12 src/flowcrypt_data.rs                     # header + FLOWCRYPT_PROVENANCE constant
grep -c '0x' src/flowcrypt_data.rs                 # entry count, expect a few thousand
grep FLOWCRYPT_PROVENANCE src/flowcrypt_data.rs    # commit + date present, non-"unknown" ideally
```
If the count is 0 or the file is malformed, STOP — the input path/schema is wrong; report before proceeding. If the GitHub API was unreachable and the commit resolved to `unknown`, re-run with an explicit `--source-commit <sha>` (look up the current `master` SHA of FlowCrypt/idn-homographs-database) so provenance is accurate — do NOT ship `unknown`.

- [ ] **Step 3: Add a failing test for the flowcrypt source**

First, pick a concrete look-alike from the generated table to test with. Find one whose target is a simple ASCII letter:
```bash
grep -m5 '"a")' src/flowcrypt_data.rs   # entries mapping some char -> "a"
```
Note one entry's code point (e.g. `0xXXXX -> "a"`). In `src/confusables.rs` tests, add (replace `\u{XXXX}` with the real code point you found, and the partner letter):
```rust
    #[test]
    fn flowcrypt_source_collapses_lookalike_when_enabled() {
        // <CODEPOINT> is a FlowCrypt look-alike of ASCII 'a' (from flowcrypt_data.rs).
        let look = '\u{XXXX}';
        let off = ConfusableMap::uts39();
        let on = ConfusableMap::from_sources(&Sources { flowcrypt: true, digraph: false });
        // Off by default: the look-alike is NOT confusable with 'a'.
        assert!(!off.confusable(look, 'a'));
        // On: it collapses to the same skeleton as 'a'.
        assert!(on.confusable(look, 'a'));
    }

    #[test]
    fn uts39_wins_over_flowcrypt_on_collision() {
        // A char defined by BOTH resolves to the UTS#39 skeleton. Cyrillic а
        // (U+0430) is a UTS#39 confusable of 'a'; with flowcrypt on it must still
        // skeleton to 'a' (uts39 precedence), never something else.
        let on = ConfusableMap::from_sources(&Sources { flowcrypt: true, digraph: false });
        assert_eq!(on.skeleton("\u{0430}"), on.skeleton("a"));
    }
```
Run: `cargo test --lib flowcrypt 2>&1 | head -20` → FAIL (from_sources ignores flowcrypt; look-alike not yet merged).

- [ ] **Step 4: Wire `flowcrypt` into `from_sources` with UTS#39 precedence**

In `src/main.rs` add `mod flowcrypt_data;` (sorted). In `src/confusables.rs` `from_sources`, merge flowcrypt entries for keys UTS#39 doesn't define, then re-sort:
```rust
    pub fn from_sources(sources: &Sources) -> Self {
        let mut singles: Vec<(u32, &'static str)> = CONFUSABLES.to_vec();
        if sources.flowcrypt {
            // UTS#39 wins: only add a flowcrypt key absent from UTS#39.
            for &(cp, sk) in crate::flowcrypt_data::FLOWCRYPT {
                if singles.binary_search_by(|&(k, _)| k.cmp(&cp)).is_err() {
                    singles.push((cp, sk));
                }
            }
            singles.sort_by_key(|&(k, _)| k);
        }
        let digraphs: &'static [(&'static str, &'static str)] = if sources.digraph {
            crate::digraph_data::DIGRAPHS
        } else {
            &[]
        };
        ConfusableMap { singles, digraphs }
    }
```
(Note: the `binary_search` against the still-UTS#39-sorted `singles` is valid because we only push absent keys and sort once at the end. Since we mutate `singles` while searching it, collect the additions first to avoid searching a half-mutated vec: build a `Vec` of to-add pairs, then extend + sort. Adjust:)
```rust
        if sources.flowcrypt {
            let mut add: Vec<(u32, &'static str)> = Vec::new();
            for &(cp, sk) in crate::flowcrypt_data::FLOWCRYPT {
                if singles.binary_search_by(|&(k, _)| k.cmp(&cp)).is_err()
                    && !add.iter().any(|&(k, _)| k == cp)
                {
                    add.push((cp, sk));
                }
            }
            singles.extend(add);
            singles.sort_by_key(|&(k, _)| k);
        }
```

- [ ] **Step 5: Run + gates**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: the 2 flowcrypt tests pass; UTS#39-precedence test passes; default behavior unchanged; all green. (If `flowcrypt_data.rs` fails `cargo fmt --check` because the generator's formatting differs, run `cargo fmt` once — generated Rust should still be fmt-stable; the simple `(0xXXXX, "..."),` rows are.)

- [ ] **Step 6: Update Cargo.toml include + commit**

In `Cargo.toml`, the `include` list has `"/scripts/gen_confusables.py"`. Add `"/scripts/gen_flowcrypt.py"` right after it (so the crate package ships the generator, mirroring the existing one).

```bash
git add scripts/gen_flowcrypt.py src/flowcrypt_data.rs src/confusables.rs src/main.rs Cargo.toml
git commit -m "$(cat <<'EOF'
feat(confusables): add the flowcrypt source (single-char supplement)

gen_flowcrypt.py filters the FlowCrypt MIT homograph DB to ASCII-base
look-alikes, anchors each to its partner's UTS#39 skeleton, dedups, and emits
src/flowcrypt_data.rs. from_sources merges it under --confusables=flowcrypt
for keys UTS#39 doesn't define (UTS#39 wins). Default behavior unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: CLI `--confusables` parsing + threading the real map + `--help`

Wire `--confusables` into the arg parser and `Opts`, build the `ConfusableMap` once in `main`, and thread it into every real `score_pair` call (replacing the `uts39()` placeholders from Task 1). Add the `--help` line.

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Failing tests for `--confusables` parsing**

Add to the `src/main.rs` `tests` module:
```rust
    #[test]
    fn parse_confusables_flag() {
        let o = parse_from(vec![
            "--confusables".into(), "uts39,digraph".into(), "a".into(), "b".into(),
        ]).unwrap();
        assert!(o.sources.digraph && !o.sources.flowcrypt);
    }

    #[test]
    fn parse_confusables_default_is_uts39() {
        let o = parse_from(vec!["a".into(), "b".into()]).unwrap();
        assert_eq!(o.sources, confusables::Sources::default());
    }

    #[test]
    fn parse_confusables_rejects_unknown() {
        assert!(parse_from(vec![
            "--confusables".into(), "uts39,nope".into(), "a".into(), "b".into(),
        ]).is_err());
    }
```
Run: `cargo test --lib parse_confusables 2>&1 | head -20` → FAIL (`Opts` has no `sources`; no `--confusables` arg).

- [ ] **Step 2: Add `sources` to `Opts` + parse `--confusables`**

In `src/main.rs`: add `sources: confusables::Sources` to the `Opts` struct; initialize `sources: confusables::Sources::default()` in `parse_from`'s `opts` initializer; add the arg arm (next to `--fields`):
```rust
            "--confusables" => {
                let v = args.next().ok_or("--confusables needs a value")?;
                opts.sources = confusables::parse_sources(&v)?;
            }
```

- [ ] **Step 3: Build the map once in `main` and thread it**

In `fn main()`, after `opts` is parsed and before the mode dispatch, build:
```rust
    let cmap = confusables::ConfusableMap::from_sources(&opts.sources);
```
Replace the THREE real `score_pair(...)` calls (the `uts39()` placeholders from Task 1 at the list-mode candidate, the stdin-batch pair, and the single-pair) with `&cmap`:
- list mode candidate: `score_pair(string, line, &cmap)` (in `score_candidate` — see note)
- stdin batch: `score_pair(la, lb, &cmap)`
- single pair: `score_pair(a, b, &cmap)`

NOTE on `score_candidate`/`process_list`: these helpers call `score_pair` internally. Thread `&cmap` through them too — add a `cmap: &confusables::ConfusableMap` parameter to `score_candidate` and `process_list` and pass it down, and update their call sites in `main` and in their tests (the tests can pass `&confusables::ConfusableMap::uts39()`). Update the `score_candidate`/`process_list` test call sites in the `tests` module to pass `&cmap()` (the helper added in Task 1).

- [ ] **Step 4: Add the `--confusables` line to `--help`**

In `print_usage`, add after the `--fields` line:
```
         \x20       --confusables <LIST> Confusable sources for skeletons: uts39,flowcrypt,digraph (default uts39)\n\
```
(Keep alignment with the surrounding `\x20`-prefixed lines.)

- [ ] **Step 4b: Add the FlowCrypt `data:` provenance line to `-v`**

Task 0 added the UTS#39 `data:` line to the `-v`/`--version` arm. Now `flowcrypt_data` exists, so add its line directly after the UTS#39 one in that arm:
```rust
                println!("  data: {}", confusables_data::CONFUSABLES_PROVENANCE);
                println!("  data: {}", flowcrypt_data::FLOWCRYPT_PROVENANCE);
```
Add a test to `src/main.rs` `tests`:
```rust
    #[test]
    fn version_includes_flowcrypt_provenance() {
        let p = flowcrypt_data::FLOWCRYPT_PROVENANCE;
        assert!(p.contains("FlowCrypt"), "names the source: {p}");
        assert!(p.contains("retrieved"), "carries a retrieval date: {p}");
        assert!(!p.contains("unknown"), "commit must be resolved, not 'unknown': {p}");
    }
```
(The `!unknown` assertion enforces that the committed table carries a real commit SHA — if it fails, the generator was run without network and without `--source-commit`; re-run Task 3 Step 2 with an explicit `--source-commit`.)

- [ ] **Step 5: Run + gates + end-to-end smoke**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` → all pass.
Smoke:
```bash
cargo build
./target/debug/sqdist -v                                                                              # 3 lines: version + 2 data: lines
./target/debug/sqdist -j devflovv devflow | grep -o '"confusable_only":[a-z]*'                       # false (default)
./target/debug/sqdist -j --confusables uts39,digraph devflovv devflow | grep -o '"confusable_only":[a-z]*'  # true
./target/debug/sqdist --confusables uts39,bogus a b; echo "exit=$?"                                   # error, exit 2
```
Expected: `-v` prints `sqdist 0.3.0 (<sha>)` then `  data: UTS#39 ...` then `  data: FlowCrypt ... @ <sha> (retrieved ...)`; default → `confusable_only:false`; `--confusables=uts39,digraph` → `confusable_only:true` (vv/w now collapses); unknown source → usage + exit 2.

> NOTE: `--confusables` must be parsed BEFORE positionals in the arg loop the same way other valued flags are — confirm `-j --confusables uts39,digraph A B` and `A B --confusables uts39,digraph` both work (the parser is order-independent for flags). If `-j` before `--confusables` causes an issue, it won't — they're independent arms.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat(cli): add --confusables source selector, thread the map, flowcrypt provenance

Parse --confusables=<list> into Opts.sources (default uts39); build the
ConfusableMap once in main and thread it through score_pair / score_candidate
/ process_list into every PairContext. --confusables=uts39,digraph now makes
devflovv/devflow read as confusable_only; default stays pure UTS#39. Add the
FlowCrypt data: provenance line to -v alongside the UTS#39 one.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Docs

**Files:**
- Modify: `README.md`, `CLAUDE.md`

- [ ] **Step 1: Update README.md**

Add a `--confusables <LIST>` entry to the OPTIONS block (matching the `--help` text). Add a short "Supplemental confusables" subsection: the three sources (`uts39` always on / default; `flowcrypt` MIT single-char supplement; `digraph` 4 curated mappings `vv/cl/rn/nn`); the **default is pure UTS#39** (authoritative); supplemental sources are **opt-in and less authoritative** (`cl→d` is high-FP: `clear`/`dear`); example `--confusables=uts39,digraph` closing the `devflovv`/`devflow` gap; and the FlowCrypt regeneration command (`python3 scripts/gen_flowcrypt.py`). Note the skeleton axes' behavior depends on `--confusables`. Add FlowCrypt MIT attribution. Also document that **`-v` reports the embedded data-source provenance** (UTS#39 version+date and the FlowCrypt repo commit+retrieval date) on indented `data:` lines, and show the 3-line `-v` example.

- [ ] **Step 2: Update CLAUDE.md**

In the architecture/file list: add `src/confusables.rs` (the runtime `ConfusableMap` model + `--confusables` source parsing), `src/digraph_data.rs` (4 curated digraphs), `src/flowcrypt_data.rs` (generated FlowCrypt supplement), and `scripts/gen_flowcrypt.py`. Note `distance.rs` no longer holds the skeleton model (moved to confusables.rs; it keeps the edit-distance algorithms + alignment). Document `--confusables=<list>` and that `skeleton_of`/`skeleton`/`confusable` are now `ConfusableMap` methods taking the active source set, threaded through `PairContext`. Add a "Regenerating the FlowCrypt table" note mirroring the existing confusables-regeneration section.

- [ ] **Step 3: Verify + commit**

Run: `cargo run -- --help | grep confusables` and confirm the docs match. Run `cargo run -- --confusables uts39,digraph -j devflovv devflow` and confirm the README example is accurate.
```bash
git add README.md CLAUDE.md
git commit -m "$(cat <<'EOF'
docs: document --confusables sources, the FlowCrypt generator, the new modules

Describe the uts39/flowcrypt/digraph sources, the pure-UTS#39 default and the
opt-in/less-authoritative caveat (cl->d high-FP), the gen_flowcrypt.py
regeneration step, and the confusables.rs/digraph_data.rs/flowcrypt_data.rs
layout (skeleton model moved out of distance.rs).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Final verification

- [ ] **Step 1: Full gate**
```bash
cargo fmt --check                              # clean
cargo clippy --all-targets -- -D warnings      # no warnings
cargo test                                     # all pass
cargo build --release                          # exit 0
./target/release/sqdist -v                     # 3 lines: sqdist 0.3.0 (<sha>) + UTS#39 data: + FlowCrypt data: (touch .git/HEAD if SHA stale)
```
Confirm `-v` prints exactly: line 1 `sqdist 0.3.0 (<sha>)`, line 2 `  data: UTS#39 confusables.txt v17.0.0 (2025-07-22)`, line 3 `  data: FlowCrypt idn-homographs-database @ <sha> (retrieved <date>)` — the FlowCrypt commit must NOT be `unknown`.

- [ ] **Step 2: Behavioral spot-checks (default vs supplemental)**
```bash
B=./target/release/sqdist
# Default = pure UTS#39: vv/w gap stays.
$B -j devflovv devflow | grep -o '"confusable_only":[a-z]*'                          # false
$B -j devflovv devflow | grep -o '"skeleton_damerau":[0-9]*'                         # > 0
# digraph closes it.
$B -j --confusables uts39,digraph devflovv devflow | grep -o '"confusable_only":[a-z]*'   # true
$B -j --confusables uts39,digraph devflovv devflow | grep -o '"skeleton_damerau":[0-9]*'  # 0
# cl->d (the documented high-FP case) collapses under digraph.
$B -j --confusables uts39,digraph clear dear | grep -o '"confusable_only":[a-z]*'    # true (the FP tradeoff, by consent)
# flowcrypt: a non-ASCII look-alike collapses (use a real codepoint from flowcrypt_data.rs).
# unknown source rejected.
$B --confusables uts39,bogus a b; echo "exit=$?"                                     # exit 2
# default panel for an ASCII typo unchanged.
$B -j gigle gogle | grep -o '"keyboard_distance":[0-9.]*'                            # still works (other axes intact)
```

- [ ] **Step 3: Spec test-checklist cross-check** — every item in the spec's Testing section has a passing test: default==old (ported tests), source parsing (default/all/unknown/dedup/empty), digraph closes gaps when on / stays off by default, longest-match-first, flowcrypt collapses when on, UTS#39 precedence, end-to-end `--confusables` CLI. Add any missing one (failing → fix → pass), commit separately.

- [ ] **Step 4: Report completion** — summarize; confirm the gate passed; nothing pushed/released. This is the LAST planned phase (Phases 2–4 of the redesign are done; CJK is permanently deferred). Note to the owner that all four phases are complete and unreleased on `main`, ready for review/release when they choose.

---

## Self-review notes (for the executor)

- **Spec coverage:** UTS#39 provenance retrofit + `-v` framework (T0); ConfusableMap refactor + default-unchanged (T1); digraph source+data (T2); flowcrypt generator+data+source+UTS#39-precedence+`FLOWCRYPT_PROVENANCE` (T3); `--confusables` CLI + threading + --help + the FlowCrypt `-v` line (T4); docs incl. provenance/regeneration (T5); verification incl. the 3-line `-v` (T6). FP-guards-documented-not-implemented = the comment in digraph_data.rs (T2), no task. Naming-note (keep uts39_confusable_count) = no code change. No-verdict-change / no-AxisValue-change = no task.
- **Provenance requirement (owner-added):** every external data table carries a `*_PROVENANCE` constant and `-v` prints one indented `data:` line per source (multi-line format). UTS#39 = version+date (T0); FlowCrypt = repo commit SHA + retrieval date because the repo has no releases (T3 generator resolves it; T4 prints it). The committed FlowCrypt table must carry a REAL commit, not `unknown` (the `version_includes_flowcrypt_provenance` test enforces this). digraph is hand-authored, not external → no provenance line.
- **Watch-points:** (1) T1 is the risky refactor — the ported default tests MUST pass unchanged; if any skeleton/confusable result differs, the default map isn't reproducing UTS#39. (2) Thread `&cmap` through `score_candidate`/`process_list` (T4) AND their test call sites. (3) `from_sources` flowcrypt merge: collect additions first, then extend+sort (don't binary_search a vec you're mutating). (4) The flowcrypt test (T3 Step 3) needs a REAL codepoint from the generated table — pick one mapping to a simple ASCII letter; don't hardcode a guess. (5) `gen_flowcrypt.py` may need `--homographs <path> --confusables <path>` if the env can't fetch, and `--source-commit <sha>` if the GitHub API is unreachable; if the generated table is empty OR the commit is `unknown`, STOP and resolve before committing. (6) Don't bump version (0.3.0). (7) No `#[allow(dead_code)]` — the moved free fns are deleted.
- **Idempotence:** digraph skeletons are single-pass (documented); tests use exact equality on `skeleton()` outputs, not fixed-point.
- **The big move (distance.rs → confusables.rs):** only 3 functions + 5 tests move; 3 call sites (2 in PairContext::new, 1 in the axis) + 3 test helpers update. Keep it surgical.
