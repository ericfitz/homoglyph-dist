# Performance analysis: comparison and edit-distance path

Analysis date: 2026-08-26.

Scope: edit distance, confusable/skeleton comparison, axis panel construction, and batch/list pairing. Reporting (JSON/human formatting) is out of scope.

There are no Criterion benches in the repo. Rankings below come from the cost model of the current code, not from a profile. Several items are algorithmically certain (duplicate work, recomputation, missing early-outs). Their share of wall time still depends on workload: a single pair is noise; `--string`/`--list` against a registry dump is where this matters.

Typical names here are short (≈5–30 chars). That changes the game: **heap allocations and repeated UTF-8/skeleton work often rival the DP arithmetic**, and a full 10-axis panel per candidate is far more than `--typosquat` rejection needs.

---

## Cost model of today’s comparison path

Every kept (and currently every rejected) pair goes through `prepare_pair` → `PairContext::new` → `build_panel`.

**`PairContext::new` always:**

1. `a.chars().collect()` and `b.chars().collect()` (`Vec<char>`, 4 bytes/code point)
2. `cmap.skeleton(a)` and `cmap.skeleton(b)` (each collects *another* `Vec<char>`, then binary-searches the ~6565-entry table per code point, then builds a `String`)
3. `ska.chars().collect()` / `skb.chars().collect()` (fourth and fifth char vecs)
4. `distance::align(&ca, &cb)` — **full OSA Damerau matrix** `(n+1)×(m+1)` of `u64`, then traceback into `Vec<AlignOp>`

**`build_panel` then independently:**

5. Levenshtein on originals (2-row DP, new `Vec`s)
6. Damerau on originals — **same recurrence as `align`, second full matrix**
7. Levenshtein on skeletons
8. Damerau on skeletons
9. Walk alignment for `uts39_confusable_count` (binary search per substitution)
10. `detect_restriction_level()` on both original strings (per-char identifier + script-extension tables; no ASCII fast path)
11. Keyboard distance (linear key lookup + **O(K²) `max_key_distance()` every pair**)
12. Derived axes via linear `Panel::get`

That is **five DP fills** (two of them identical), **~8–12 heap allocations**, and **no reuse of the query side** in list mode.

`ska`/`skb` are `#[allow(dead_code)]` in production; only `sva`/`svb` are used.

---

## 1. Algorithm / DP (highest density of real wins)

### 1.1 Don’t fill the OSA matrix twice

**Where:** `distance::damerau` and `distance::align` are copy-pasted fills; `PairContext` always calls `align`, then the `damerau` axis calls `damerau` on the same slices.

**Why it hurts:** For every pair you pay `O(nm)` twice plus two `(n+1)(m+1)` allocations. The Damerau distance *is* `d[n,m]` after the `align` fill.

**Strategy:** One function that fills once and returns `(distance, ops)`. Or: 3-row `damerau()` for the scalar, and only build the full matrix when alignment is actually required.

**Profiling:** Not needed to prove waste. Needed only to see % of wall time.

### 1.2 Damerau does not need a full `u64` matrix

OSA only reads rows `i`, `i-1`, `i-2`. `damerau()` can be three rolling rows (or one `(m+1)*3` buffer).

`align()` still needs history. Cheaper than a `u64` matrix:

- 2-bit predecessor per cell (≅ 32× less traffic than `u64`), or
- traceback while keeping the matrix only when `uts39_confusable_count` / `keyboard_distance` will run.

**Profiling:** Helpful for cache effects at n≳32; obvious win in allocation size for watchlists.

### 1.3 Integer width

Costs are `u64`. Distance is at most `max(n,m)`. Registry names fit in `u8` (or `u16` if you want headroom).

**Strategy:** `u8`/`u16` cells → ~4–8× less matrix traffic, better SIMD/register packing.

**Profiling:** Worth a microbench; this is a real inner-loop win, not a guess.

### 1.4 Prefix/suffix stripping (safe for Lev + OSA)

Strip the common prefix, then the common suffix, then DP the remainder.

Valid for Levenshtein. Valid for OSA too: if `a[0]==b[0]`, a transposition cannot involve index 0 unless both sides are the same repeated character (already a match). Same at the suffix.

Typosquat pairs are usually “same word, one edit in the middle” — you often DP 1–3 remaining characters.

**Profiling:** Microbench on real name pairs. Expected large win on the DP itself; smaller if allocation dominates.

### 1.5 Ukkonen / threshold cutoff

When `-t` is set, after row `i` if `min(row) > t` abort. Also: **if `|n-m| > t`, skip DP** (`damerau ≥ |n-m|`).

For `--typosquat`, classification only keeps `damerau ≤ 1` or `confusable_only` or combosquat. So `|n-m| > 1` plus skeletons unequal plus not-combosquat → **reject with no DP**.

**Profiling:** Needed on a real watchlist to quantify (almost certainly huge). The length prune itself is free and exact.

### 1.6 Specialized `k = 1` Damerau (don’t use general DP)

`--typosquat` only cares whether OSA distance is 0, 1, or ≥2.

O(n) checks:

- same length: Hamming ≤ 1, or one adjacent swap (`ab`↔`ba`)
- length differs by 1: one insert/delete (two-pointer)
- length differs by ≥2: not `damerau ≤ 1` (still need skeleton equality / combosquat)

This is the spellchecker “one-edit” algorithm. It should replace the `O(nm)` fill for the reject path.

**Profiling:** Not needed to know it’s asymptotically better. Needed before using it as the *emitted* `damerau` value for `k>1` (it isn’t; fall back to full DP only on hits you will print).

### 1.7 Bit-parallel Myers / Hyyrö (n, m ≤ 64 or 128)

Package/domain lengths are almost always ≤ 64. Myers’ bit-vector Levenshtein is typically an order of magnitude faster than row-by-row DP on that range. There are OSA/Damerau bit-parallel variants (Hyyrö et al.).

ASCII: 256-entry PEQ table. Unicode: PEQ keyed by the distinct code points in the pattern (usually ≪ 64).

**Strategy:** Specialize:

| length | kernel |
|---|---|
| 0 / equal | immediate 0 |
| ≤ 16 | stack DP, `u8`, maybe fully unrolled |
| ≤ 64 | Myers / Hyyrö bit-parallel |
| longer | banded DP |

**Profiling:** Required. Bit-parallel vs tiny stack DP can go either way at n≈8; at n≈20–64 bit-parallel usually wins.

### 1.8 Inner-loop dependence (why unrolling/SIMD on the current loop is weak)

Levenshtein inner loop:

```text
cur[j] = min(prev[j-1] + (a[i-1]!=b[j-1]),  // sub
             prev[j]   + 1,                  // del
             cur[j-1]  + 1)                  // ins  ← loop-carried
```

The insert term depends on `cur[j-1]`, so **you cannot SIMD or unroll independently across `j`**. That is why Myers (bits = many `j` in one register) and anti-diagonal evaluation exist.

Still worth doing, no profile needed to justify the transform:

- Peel row `i=1` and column `j=1` so the OSA `i>1 && j>1` test vanishes from the inner loop
- Make transposition **branchless** (`cmov` / mask) — that `if` is data-dependent and mispredicts on mixed typo/spoof batches
- Walk row pointers instead of `idx = i*cols+j` (compiler often does this; don’t assume)
- After an explicit `len` assert, `get_unchecked` on `a`, `b`, `prev`, `cur` — bounds checks on four indexings per cell are a known 10–30% on DP microbenchmarks
- Evaluate on anti-diagonals if you insist on SIMD without Myers

**Profiling:** Godbolt + `criterion` on the inner kernel. Don’t unroll the current `j` loop and expect a miracle.

### 1.9 Fused Levenshtein + Damerau

One pass, two accumulators (with/without transposition). Shares `a[i]!=b[j]` and loads. Saves a full 2-row Levenshtein fill when both axes are required.

**Profiling:** Nice-to-have after (1.1). Small compared to dropping unused axes.

### 1.10 Don’t zero the matrix

`vec![0u64; (n+1)*cols]` zeroes memory you immediately overwrite. `with_capacity` + write every cell (you already do) avoids the zero pass.

**Profiling:** Micro; visible when n is large or pair rate is huge.

### 1.11 Put the short string on the inner axis

Always iterate so `m = min(n,m)`. 2-row Levenshtein memory becomes `O(min)` and the inner loop is the shorter one (better when one name is an affix of the other).

---

## 2. Pair orchestration — compute far more than you use

### 2.1 Always-on 10-axis panel

`build_panel` always runs every base axis, including `script_restriction` and both skeleton DPs, even under `--typosquat` (emit set is 5 axes) and even under `--fields`.

Documented that `-t`/`--metric`/`--sort` use full internal scores — so the needed set is `fields ∪ {metric} ∪ verdict/classify deps`, **not** always `ALL_AXES`.

`--typosquat` reject path needs:

- identity, optional normalize/same_project
- combosquat (string-only)
- `confusable_only` ≡ `a!=b && skeleton(a)==skeleton(b)` (no skeleton DP)
- `damerau ≤ 1` (k=1 checker)
- full panel **only for rows you emit**

Default JSON still wants all 10 — but stdin/list with `-t` can skip everything after the metric is known to miss, and can skip axes not in `fields ∪ metric`.

**Profiling:** Required to pick the cutoff; the wasted `unicode-security` + extra DPs on a 500k list are obvious.

### 2.2 No identity short-circuit

`Equal` is `ctx.a == ctx.b` *after* building skeletons, alignment, and four DPs that are all zero.

**Strategy:** If `a == b` (memcmp), return a constant panel. Watchlists often include the package itself.

**Profiling:** Not needed.

### 2.3 Watchlist recomputes the query every line

`score_candidate` → `prepare_pair(string, line, …)` redoes, for the **same** `--string`:

- normalize
- `chars().collect()`
- skeleton
- skeleton chars

**Strategy:** A `PreparedQuery` (normalized form, char vec, skeleton vec, ASCII flag, restriction level of the query). Each candidate only prepares itself, then DPs.

**Profiling:** Not needed to prove duplication. Magnitude = list length.

### 2.4 Allocation storm

Per pair, heap:

`ca`, `cb`, `ska`, `skb`, `sva`, `svb`, `align`, Lev `prev`/`cur`, Damerau full matrix, align full matrix, skeleton Lev, skeleton Damerau, plus `prepare_pair`’s `to_string()` copies of `a`/`b`/`sa`/`sb`.

For n≈10, **malloc can beat arithmetic**.

**Strategy:**

- Thread-local / passed-in scratch (`Vec`s cleared, capacity kept)
- Stack arrays for `n,m ≤ 64` (`[u16; 65*65]` is ~8KB — fine)
- Skeleton **into** `Vec<char>` or `[u32; 64]`; drop unused `ska`/`skb` `String`s
- Store only `AlignOp::Sub` (or just `(i,j)` pairs); don’t materialize Match/Ins/Del
- List mode: don’t clone the query into every `Row` (`Arc<str>` or omit)

**Profiling:** `dhat` / Instruments allocations. Do this **before** bit-parallel; at n=8 it may be the whole story.

### 2.5 Trait objects + `Panel::get` + reorder

10 virtual calls, linear get on 10 entries, clone snapshot, `sort_by_key` scanning `ALL_AXES`.

**Ignore.** Not nontrivial.

### 2.6 Classify twice in batch `--typosquat`

`keep_scored` calls `classify_typosquat`, then `emit_row_json` → `class_from_row` does it again (including combosquat allocations).

Reporting-adjacent; cheap vs DP. Easy to cache on the row if you touch this path.

---

## 3. Confusable / skeleton path

### 3.1 `CONFUSABLES.to_vec()` on every run

```rust
pub fn from_sources(sources: &Sources) -> Self {
    let mut singles: Vec<(u32, &'static str)> = CONFUSABLES.to_vec();
```

Default `uts39()` copies ~6565 `(u32, &str)` to the heap (~80KB) then binary-searches the copy. The static slice is already sorted.

**Strategy:** `Cow<'static, [(u32, &str)]>` or an enum `Static(&'static […]) | Merged(Vec<…>)`. Merge only when `flowcrypt` is on.

**Profiling:** Startup only for CLI; do it anyway (correctness-preserving, zero risk).

### 3.2 Binary search per code point (~13 comparisons)

Table: 6565 entries, **8 ASCII / 26 Latin-1 / 4444 BMP / 2121 astral**.

Default package names are ASCII. `skeleton("lodash")` today: collect 6 chars, 6 binary searches through a mixed BMP+astral table, build a `String` that equals the input.

**Strategy (stack, cheapest first):**

1. **ASCII lookup table** (128 or 256). Only 8 ASCII source CPs are mapped (`0→O`, `1→l`, `I→l`, `m→rn`, …). If a byte is unmapped, copy through.
2. **ASCII “no mapped char” fast path:** if none of those 8 bytes appear, `skeleton(s) ≡ s` (memcpy / borrow). `confusable_only` becomes `a!=b && a==skel` only when mapped chars exist.
3. **Streaming skeleton equality** without allocating `String`s (two mapped iterators). Enough to accept/reject `confusable_only`.
4. BMP: `u16` index table (128KB) or page tables — O(1) for Cyrillic/Greek homoglyphs. Tradeoff vs the current “small binary” release profile.
5. SoA: dense `&[u32]` keys for binary search (26KB, L1-friendly), values in a side table. Helps non-ASCII only.

**Profiling:** (1)+(2)+(3) are certain wins on ASCII watchlists. (4) needs a homoglyph-heavy bench and a binary-size budget.

### 3.3 `skeleton()` allocates a `Vec<char>` and under-capacity `String`

`m→rn` expands. `with_capacity(s.len())` can realloc.

Digraph mode (opt-in but ugly):

```rust
let window: String = chars[i..i + klen].iter().collect();
if window == src {
```

Three digraphs × every position × `String` from a 2-char window. All current keys are length-2 ASCII.

**Strategy:** Compare `chars[i]=='v' && chars[i+1]=='v'` (or `s.as_bytes()` windows). Pre-sort by length once at map build. Skip the digraph loop entirely when `digraphs.is_empty()` — LLVM may already, don’t rely on it in the ASCII memcpy path.

**Profiling:** Only when `--confusables=digraph`. The `Vec<char>` in the default path is always on.

### 3.4 `confusable()` does two binary searches + UTF-8 encode fallbacks

Used per aligned substitution (usually 0–3). Minor next to DP. Still: map both CPs through the ASCII/BMP table and compare `&str` pointers or interned ids.

If two unmapped chars are equal you already returned `true`. If unmapped, compare the chars, not their UTF-8 encodings.

### 3.5 `confusable_only` via `skeleton_levenshtein == 0`

That is `sva == svb`. You already have the skeletons. The extra skeleton Levenshtein DP is redundant **for this boolean**. You still want the numeric axes for default output; on the `--typosquat` reject path you do not.

---

## 4. Keyboard, script, normalize, combosquat

### 4.1 `max_key_distance()` is O(K²) **per pair**

~47²/2 `sqrt`s every time `keyboard_distance` runs. The value is a constant of the table.

**Strategy:** `const MAX: f32 = …` (or `OnceLock`, or `f32` computed in a `#[test]` that pins the value).

**Profiling:** Not needed.

### 4.2 `key_coord` linear-scans row strings

**Strategy:** `[Option<(f32,f32); 128]` for ASCII. You already NA-out non-ASCII.

`to_ascii_lowercase` + two lookups + `powi`/`sqrt` per substitution is fine once the table is O(1) and the max is const. Could skip the axis when there are zero `Sub`s (identical / pure ins/del) — already returns 0, but after walking align.

### 4.3 `script_restriction` has no ASCII fast path

`unicode-security`’s `detect_restriction_level` still walks every char through `identifier_allowed` and `ScriptExtension` **before** returning `ASCIIOnly`.

You already know `str::is_ascii()` (SIMD in std). Tests even pin ASCII → 0.

**Strategy:** `if ctx.a.is_ascii() && ctx.b.is_ascii() { return 0; }`
If one is ASCII, only run the crate on the other.

**Profiling:** Not needed for the short-circuit. Needed if you want to replace the crate on the mixed-script path (probably not worth it).

### 4.4 Normalize is per-candidate and multi-pass

`apply_ops`: each op allocates a new `String`. PEP 503 is lower + map `._-`→`-` + collapse `-`. Charset membership is `charset.chars().any()`.

For `--pypi` on a watchlist:

- lower+map+collapse in **one** pass (or two: ASCII lowercase in place, then delimiter fold)
- charset as a `[bool; 128]` / `u128` bitmap
- pre-normalize the query once (2.3)

**Profiling:** Only if `--pypi`/`-n` is on the hot list path. Idle when ops are empty (`on.then(|| …)` is already good).

### 4.5 `is_possible_combosquat` allocates

`to_lowercase()` of both strings, `format!("{short}{sep}")` per separator, then `split`.

**Strategy:** ASCII: `eq_ignore_ascii_case`, `starts_with`/`ends_with` with `sep` checked at the boundary without building prefix strings. Unicode lowercase only if non-ASCII.

**Call order for `--typosquat`:** combosquat **before** DP for long-name rejects? You still need the panel for *kept* combosquats. For *rejects*, combosquat must be checked or you’ll drop `lodash-utils`. So: cheap combosquat first; if true, score; if false, k=1/skeleton filters; if those fail, drop with no panel.

---

## 5. List/batch structure (the other 10×)

These are comparison-path, not reporting.

### 5.1 Linear scan of the whole list with a full panel

`--string S --list FILE` is `|FILE|` independent scores. For d=1 watchlist search the literature answer is **not** “faster DP,” it’s **don’t visit unrelated names**:

- length bucket (`|len-q| ≤ 1` plus a separate combosquat pass on names that contain `q` as a token)
- BK-tree / VP-tree
- Levenshtein automaton / DAWG (deterministic d=1)
- n-gram inverted index (filter, then exact)

Building an index of 500k PyPI names once per process is cheap relative to 500k full panels.

**Profiling:** Measure list size × pair time first. If you’re already <50ns/pair, an index is wasted; you are not.

### 5.2 Data parallelism

Embarrassingly parallel over list lines / stdin pairs. Rayon over chunks, thread-local scratch (2.4), ordered output only if you care (today list mode without `--sort` is input order).

**Profiling:** Scaling curve vs list size. Watch false sharing on the allocator until scratch is pooled.

### 5.3 Streaming reject vs `--sort`/`--top`

Without sort you already stream. With `--top N`, you don’t need to store full `Panel`s for losers: keep a size-N heap of `(metric, Row)`. Still must **compute** the metric for everyone unless you have an index or a bound.

A max-heap of size N plus Ukkonen cutoff at the current Nth metric is a standard “top-k closest strings” combo.

---

## 6. What not to touch (for speed)

- `Panel::get` / `ALL_AXES` trait objects / canonical reorder
- JSON/human formatting (out of scope)
- FlowCrypt merge (once at startup, 477 entries)
- True Damerau (current is OSA; full DL is slower and would change answers)
- GPU / blocking / huge pages (n is tiny)
- Replacing UTS#39 semantics (`0`~`O` but not `0`~`o`) in a “faster” map

---

## Suggested attack order (if implementing)

1. **Add a watchlist bench** (e.g. 100k realistic names × one query, ASCII + a homoglyph query). Without this you will optimize the DP kernel and miss the alloc/skeleton/reject path.
2. **Certain, semantics-preserving cleanups** (do not need a profile to justify): identity short-circuit; reuse `align`’s last cell as Damerau; don’t copy `CONFUSABLES`; const `max_key_distance`; O(1) `key_coord`; ASCII `script_restriction`; drop dead `ska`/`skb`; precompute query side in list mode.
3. **`--typosquat` / `-t` reject path:** combosquat → streaming skeleton equality → `|n-m|` prune → O(n) k=1 Damerau → full panel only on hits.
4. **Allocation:** stack/scratch buffers for n≤64; skeleton into chars without intermediate `String`.
5. **Then** measure whether DP still shows up. If yes: prefix/suffix strip, `u8` cells, 3-row OSA, peeled inner loop, unchecked indexing, then Myers/Hyyrö.
6. **If list mode is the product:** length buckets / automaton, then rayon.

---

## Verification vs guess

| Opportunity | Profile needed to know it exists? | Profile needed to know it’s the bottleneck? |
|---|---|---|
| Duplicate Damerau fill (`align` + `damerau`) | No | Yes (share of total) |
| Full panel on every watchlist miss | No | Yes |
| Query-side recomputation in `--list` | No | Yes |
| `CONFUSABLES.to_vec()` | No | No (startup) |
| `max_key_distance` / linear `key_coord` | No | Yes (likely small after DP/alloc) |
| ASCII skeleton via 13-way binary search | No | Yes |
| Digraph `String` windows | No | Only with `digraph` on |
| Identity / `|n-m|` / k=1 early-out | No | Yes (likely #1 on `--typosquat` lists) |
| Heap allocs at n≈8 | **Yes** (`dhat`) | Yes |
| Myers vs stack DP | **Yes** | Yes |
| `get_unchecked` / branchless trans | **Yes** | Yes |
| BMP index table vs binary search | **Yes** + size budget | Yes |
| Rayon / BK-tree / automaton | **Yes** (list size) | Yes |

The shape of a fast comparator, given this codebase, is: **ASCII/query specialization and reject-before-DP**, not a cleverer `min(sub,del,ins)` in the general Unicode kernel. The general kernel still has real fat (duplicate fill, `u64` full matrix, bounds checks, transposition branch), but it should be the second thing you speed up, after you stop running it 500,000 times on names that cannot match.
