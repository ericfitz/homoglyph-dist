# Research findings: candidate new axes for sqdist

Date: 2026-05-23
Status: Research spike (informs later per-axis design specs)
Context: The approved multi-axis architecture
(`2026-05-23-multi-axis-architecture-design.md`) defers all new signal axes to
their own specs. This doc captures the research spike for the candidate axes, to
decide which are worth designing and how.

## Constraints applied

- **HARD FILTER:** any data/library must be redistribution-compatible with an
  `MIT OR Apache-2.0` binary. (Unicode License v3 = SPDX `Unicode-3.0`, OSI-
  approved, MIT-like, compatible with attribution. CC-BY-4.0 = usable but needs
  a carried NOTICE. No-license repos = unusable.)
- **PREFERENCE:** keep the std-only / embedded-generated-table approach; allow a
  redistribution-licensed dependency only where rolling our own is clearly
  inferior. Binary size reported, not gating.

## Summary recommendation

| Axis | Recommendation | Effort | Notes |
|---|---|---|---|
| **script-count (mixed-script)** | **Do — highest value** | Small (crate) / Medium (DIY) | Directly catches the `pаypal` class. Use UTS#39 restriction level. |
| **keyboard-distance (kbd)** | **Do — good value, self-contained** | Small | Hand-built QWERTY coordinate table; no usable crate exists. |
| **digraph / supplemental confusables** | **Do — with strong FP guards** | Small–Medium | `rn↔m`, `vv↔w` low-risk; `cl↔d` high false-positive risk — gate it. |
| **CJK pseudo-homoglyph** | **Defer** | Medium | Low practical value (registries block CJK names); noisy/open-ended. |

Suggested sequencing: **script-count first** (biggest detection win, clean data),
then **keyboard-distance** (self-contained, small), then **digraph/supplemental**
(needs FP tuning). CJK deferred indefinitely pending a real use case.

---

## Axis 1 — script-count (mixed-script detection) — DO FIRST

**What it detects:** strings whose characters span multiple Unicode scripts that
can't resolve to one consistent script — the mechanism behind `pаypal` (Latin
`a` → Cyrillic `а`). This is arguably the single most direct homoglyph-spoof
signal and is currently absent from sqdist.

**Definition (recommended):** the **UTS#39 restriction level** (an integer
0–5), derived from the *resolved script set* (UTS#39 §5.1–5.2):
- Augment each char's `Script_Extensions`: `Han` adds synthetic `Jpan`/`Hanb`/
  `Kore`; `Common`/`Inherited` expand to ALL.
- Resolved set = intersection across all chars. Non-empty ⇒ single-script;
  empty ⇒ mixed-script.
- Levels: 0 ASCII-only, 1 single-script, 2 highly-restrictive (Jpan/Kore/Hanb),
  3 moderately, 4 minimally, 5 unrestricted.

**Critical correctness point:** a naive "count of distinct Script values" axis
would FALSE-POSITIVE on legitimate Japanese (Han+Hiragana+Katakana = 3 scripts).
The augmented-set algorithm collapses those to `{Jpan}` (level 2, single). So we
must use the resolved-set/restriction-level model, not a raw count. Korean
(Hangul+Han→Kore) and Bopomofo+Han (Hanb) are likewise handled.

**Data/impl — two options:**
- **A (recommended): `unicode-security` crate** (MIT OR Apache-2.0, unicode-rs
  org, `no_std` capable; pulls `unicode-script` ~255 KB table +
  `unicode-normalization`). Exposes `RestrictionLevelDetection` — the exact
  algorithm, already validated as the basis of rustc's `mixed_script_idents`
  lint. The subtle augmented-set logic is easy to get wrong by hand, so for a
  security tool the crate is the right call.
- **B (DIY): generate an embedded table** from `Scripts.txt` +
  `ScriptExtensions.txt` (Unicode-3.0 license; ~5–15 KB compressed range table)
  via the existing generator pattern or `ucd-generate`, then implement the
  augmented-set intersection (~50–100 lines). Keeps the zero-dep property but
  risks edge-case bugs.

This is the one axis where the research **recommends accepting a dependency**
(option A) over DIY, per our "deps only when clearly worth it" rule — the
correctness stakes and the subtlety of the algorithm justify it. A design spec
should make the A-vs-B call explicitly.

**Axis value shape:** Int restriction level. Direction: HigherMoreDifferent
(higher level = more suspicious). The verdict could treat level ≥ 2 (and
especially a Latin+Cyrillic/Greek mix) as a strong spoof signal.

**FP risk:** low with the restriction-level model. Legit CJK handled. Pure
Common/Inherited (digits, punctuation, emoji) never inflate it.

Sources: UTS#39 (tr39), UAX#24 (tr24), Scripts.txt/ScriptExtensions.txt,
`unicode-security` + `unicode-script` crates (unicode-rs), `ucd-generate`.

---

## Axis 2 — keyboard-distance (kbd) — DO, SELF-CONTAINED

**What it should mean (the open question, resolved):** mean physical key
distance over the *substituted* positions in the alignment, on a **stagger-aware
US-QWERTY** coordinate grid, normalized to [0,1]. Near 0 = substitutions are
adjacent-key fat-finger candidates (benign typo); near 1 = far-apart, deliberate.

Definitions surveyed and rejected: binary adjacency (too coarse — dnstwist uses
it for *generating* typos, not scoring); probabilistic fat-finger confusion
matrices (corpus-dependent, proprietary, too heavy); image/CNN approaches (out of
scope for a CLI). The continuous Euclidean-on-coordinates model retains gradient
and embeds trivially.

**Data:** **no redistribution-licensed coordinate dataset worth depending on**
(crates.io "keyboard" = hardware drivers only; urlcrazy is proprietary). Hand-
build a ~44-key QWERTY `char → (f32,f32)` table with row-stagger offsets
(number 0.0, top +0.5, home +0.75, bottom +1.25 key-units). Physical key
positions are factual, not copyrightable. `clavier` (MIT) and dnstwist
(Apache-2.0) usable as cross-references. Stays fully self-contained — fits the
preferred approach with no dependency.

**Axis value:** Float in [0,1] = mean key-distance over substitutions / max
single-key distance. Direction: HigherMoreDifferent. Return 0.0 when there are
no substitutions (purely insert/delete or identical) — document as uninformative
in that case.

**FP / edge cases:** lowercase + ASCII-fold before lookup (case = same key);
include digits + common symbols; **skip the axis (emit None/NaN, not 0.0) when
either string has non-ASCII** — keyboard distance is undefined there and 0.0
would falsely imply proximity. (This means the axis can be *absent* for some
pairs — the architecture's `AxisValue` may need an "N/A" representation; flag for
the design spec.) QWERTY-only v1 is defensible; the "Smörgåsbord" paper (IEEE SPW
2019) shows attackers exploit AZERTY/QWERTZ too, so a `--layout` flag is a future
extension, not v1.

Sources: clavier (MIT), dnstwist (Apache-2.0), "A Smörgåsbord of Typos" (Le
Pochat et al., IEEE SPW 2019), the DiVA weighted-edit-distance thesis.

---

## Axis 3 — digraph / supplemental confusables — DO WITH FP GUARDS

**What it adds:** the pure-ASCII multi-char visual confusions UTS#39 does NOT
define — `rn↔m` (only this is in UTS#39, via m's skeleton), plus `vv↔w`,
`cl↔d`, `nn↔m`, and Latin ligatures NFKC catches but confusables.txt omits
(`fi`, `fl`, `ff`, `oe`, `ae`, `ij`, ...). This is the gap that made
`devflovv`/`devflow` read as benign.

**Authoritative? No — inherently curated/heuristic, BUT standards-acknowledged.**
No standard *enumerates* ASCII digraph confusables, and (key new finding) **no
source empirically validates them with a human-perception study** — the
ShamFinder MTurk study and every pixel-similarity dataset are single-character
only. However, the *phenomenon* is explicitly acknowledged by the Unicode
standard: **UTR#36 §2.3 ("Single-Script Spoofing")** states verbatim that "the
sequence 'rn' ... is visually confusable with 'm' in many sans-serif fonts," and
**UTS#39 §5.4** names "detecting two distinct sequences that have identical
representations" as a recognized gap / optional future enhancement not covered by
the current data files. So a digraph axis fills a hole the standard itself flags
— but the standard provides **no data** for it.

The best citable data precedent remains **dnstwist's `glyphs_ascii`**
(Apache-2.0): a ~48-entry table containing exactly `'rn':('m',)`, `'cl':('d',)`,
`'vv':('w',)`, `'m':('n','nn','rn')`, etc. Directly reusable with attribution.
Other homoglyph libs (confusable_homoglyphs, life4/homoglyphs, codebox/homoglyph,
**FlowCrypt idn-homographs-database** [MIT, ~13K pairs], **ShamFinder SimChar**
[no license — unusable]) are all **single-char only** and add NO digraph pairs —
useful only for *single-char* confusable supplementation, not this axis.

**Font-dependence (new):** UTR#36 and ShamFinder both stress confusability is a
rendering artifact. `rn`→`m` specifically is a *proportional-font kerning*
effect (the two glyphs visually merge) and is largely absent in monospace/bitmap
fonts. This argues for a short-name / context gate rather than blanket matching.

**The false-positive problem (must be designed around):**
- `cl→d` is a **minefield**: `clear`/`dear`, `clock`/`dock`, `clap`/`dap`. Common
  English prefixes. High FP rate on real package names.
- `rn→m` medium risk; `vv→w` low (`vv` rare). `nn→m` speculative.
- Mitigations the design must adopt: gate digraph matching to when the
  surrounding skeleton edit distance is tiny (≤1 sub); only ever flag against
  *real registry names* (finite set), not arbitrary strings; a minimum-length
  floor; a short-name / font-context gate (digraph merging is a small-size
  proportional-font effect — UTR#36); and a per-pair allow/deny list so `cl↔d`
  can be disabled. Precedent: libu8ident needs manual exceptions even for the
  ASCII confusables range.
- **Same-script gating (from ICANN IDN Implementation Guidelines):** the
  registry world's primary confusable-FP control is the *same-script
  requirement* — a label's code points must all come from one script, so
  cross-script confusion is prevented by exclusion. Borrowable here: digraph
  confusions like `rn`/`m` are intra-ASCII-Latin, so only apply the digraph
  check to single-script Latin strings; if the pair is already flagged
  mixed-script (Axis 1), the digraph check is redundant. (ICANN's mechanism is
  prevention-by-exclusion, not detection; as a *detector* sqdist must score/rank
  instead, but the same-script scoping principle transfers.)

**Design recommendation — merge vs separate axis:** extend the **skeleton
mechanism** with curated multi-char entries (so `rn`→`m` collapses in the
existing skeleton-based axes) rather than a wholly separate axis, BUT gate the
supplemental (non-UTS#39) entries behind an **opt-in flag** (e.g.
`--extra-confusables` / `--aggressive`) so default behavior stays pure UTS#39 —
preserving the "authoritative data" guarantee and keeping the high-FP `cl↔d`
out of the default path. This reconciles with the architecture: the skeleton
axes consume whatever the skeleton map contains; the flag controls whether
supplemental entries are in the map for a given run. Provenance (dnstwist /
Apache-2.0) recorded in the generated table.

The opt-in-flag decision is *reinforced* by the new research: because digraph
confusability is curated/heuristic, font-dependent, AND lacks any empirical
human-perception validation (unlike single-char confusables, which ShamFinder
validated via MTurk), keeping it off the default path is the conservative,
defensible choice — the pure-UTS#39 default stays evidence-backed.

**Effort:** small to import the table; medium for the longest-match-first
multi-char skeleton rewrite + FP tuning against real npm/PyPI/crates name lists.

**Possible companion (single-char, separate from digraphs):** the FlowCrypt
idn-homographs-database (MIT, ShamFinder-derived, ~13K single-char pairs,
human-validated method) could supplement the *single-char* UTS#39 coverage under
the same opt-in flag — but it's 18.4 MB and IDN/cross-script focused, so it would
need filtering to the relevant subset. Tracked as a separate possibility, not
part of the digraph axis.

Sources: dnstwist `glyphs_ascii` (Apache-2.0); **UTR#36 v15 §2.3, §2.10** (Unicode
License — acknowledges `rn`/`m`, restriction levels, allowlist/casefold/NFKC FP
controls); **UTS#39 §5.4** (sequence-confusable gap); **ICANN IDN Implementation
Guidelines 2012** (same-script requirement, variant tables — ICANN terms, data
not reused); **FlowCrypt idn-homographs-database** (MIT, single-char only);
**ShamFinder / SimChar**, Suzuki et al., IMC 2019, DOI 10.1145/3355369.3355587
(single-char only, no license — method portable but data unusable; no digraph
validation); confusables.txt v16; confusables-vs-NFKC conflict writeup;
libu8ident exceptions; TypoSmart (arXiv).

---

## Axis 4 — CJK pseudo-homoglyph — DEFER

**Honest finding: low value now, high noise.** Recommended **defer indefinitely**
pending a concrete use case.

- **Practical attack surface is near zero for package names:** npm/PyPI/crates/
  Maven/Go enforce ASCII (or near-ASCII) package names. CJK homoglyphs matter
  mainly for IDN domains (out of sqdist's package-name scope) and a few lax
  registries (NuGet).
- **UTS#39 already covers the thin relevant slice** (~90 CJK entries, mostly
  compatibility/parenthesized/radical forms). General Han-to-Han visual
  confusion is NOT in confusables.txt.
- **The space is huge and fuzzy:** ~102k Han chars; "looks similar" depends on
  font/size. No clean static table except Unihan **`kSpoofingVariant`** (349
  expert-curated, purely-visual pairs, Unicode-3.0 license) and `kZVariant`
  (149, mostly visual). ML-predicted sets (8k+ pairs) are high-noise.
- **Must NOT conflate** simplified↔traditional (`kSimplifiedVariant`/
  `kTraditionalVariant`) or semantic variants with visual confusability — those
  are not homoglyphs (爱/愛 look different).

If ever revived: scope strictly to `kSpoofingVariant` (+ maybe `kZVariant`),
Unihan, embedded table, ~500 entries, small implementation.

Sources: UAX#38 (Unihan), kSpoofingVariant.txt / kZVariant.txt (Unicode-3.0),
EquivalentUnifiedIdeograph.txt, confusable-vision (CC-BY-4.0),
ml-confusables-generator (experimental), ShamFinder (no license — unusable).

---

## Cross-cutting implications for the architecture

The research surfaced two things the multi-axis architecture spec should
accommodate when these axes are designed:

1. **Axes may be N/A for a given pair.** keyboard-distance is undefined for
   non-ASCII input. The current `AxisValue` enum (Int/Float/Bool) has no "not
   applicable" variant. The kbd design spec must either add an `AxisValue::NA`
   (and decide how JSON/human output and `--metric`/`--sort` treat it) or define
   a sentinel. Recommend adding an explicit N/A.
2. **An opt-in flag governs non-UTS#39 data.** The digraph/supplemental axis
   needs `--extra-confusables` (or similar) so default runs stay pure-UTS#39.
   This is a CLI addition the architecture didn't anticipate; its design spec
   should define the flag and how it composes with `--metric`/`--fields`.

Neither blocks the architecture; both are inputs to the per-axis specs.

## Next step

Design specs, in recommended order: (1) script-count, (2) keyboard-distance,
(3) digraph/supplemental confusables. Each its own brainstorm → spec → plan.
CJK deferred.
