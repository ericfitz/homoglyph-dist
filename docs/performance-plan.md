# Performance evaluation plan + corpus research

Companion to [performance-analysis.md](performance-analysis.md). That doc says *what* is slow by
inspection. This one says *how to measure it*, *on what data*, and *in what order* — plus the
research spike on where to get the data.

Date: 2026-08-27. All numbers and URLs below were measured/verified on that date on this machine
(Darwin 25.6.0, `sqdist` 0.5.0 release profile, single-threaded).

---

## 0. Baseline (measured, not modeled)

The analysis doc had no profile. Here is one, so the plan is anchored to real numbers.

Corpus: full PyPI simple index, **879,491 names**, ASCII-dominated.

| Command | Wall | Per pair |
|---|---|---|
| `--string requests --list pypi-names.txt -t 1` | **2.99 s** | 3.4 µs |
| `--string requests --list pypi-names.txt --typosquat` | 3.38 s | 3.8 µs |
| `--string requests --list pypi-names.txt --typosquat --pypi` | 3.71 s | 4.2 µs |
| `--string <30-char> --list pypi-names.txt -t 1` | 5.58 s | 6.3 µs |
| `--string rеquests` (Cyrillic е) `--list … -t 1` | 2.25 s | 2.6 µs |
| `--string requests --list … -t 1 --fields equal` | **3.04 s** | 3.5 µs |
| `cat pypi-names.txt > /dev/null` (I/O floor) | 0.28 s | — |

Four facts fall out immediately:

1. **~90% of wall time is compute**, not I/O or parsing. The optimization target is real.
2. **`--fields equal` costs the same as the full panel** (3.04 s vs 2.99 s, i.e. noise). This is
   direct confirmation of analysis §2.1 — the whole 10-axis panel runs on every rejected candidate.
   Single most convincing measured evidence in the set.
3. **Query length drives cost linearly-ish** (8 chars → 3.0 s, 30 chars → 5.6 s). Consistent with
   `O(nm)` DP dominating, so §1.4/§1.5/§1.6 (prefix strip, length prune, k=1 checker) have room.
4. **The Cyrillic query is 25% *faster* than the ASCII one.** Counter-intuitive, and worth
   understanding before optimizing: `keyboard_distance` NA-shortcuts on non-ASCII and skips its
   alignment walk. That means the *default ASCII path pays for an axis the non-ASCII path skips* —
   §4.1/§4.2 are on the hot path for exactly the workload we care about.

**Scale framing.** One query × PyPI = 3.3 s. One query × npm (4.39 M names) ≈ 17 s. The real
product workload — top-15k PyPI packages × all 879k names — is ~1.3 × 10^10 pairs ≈ **14 CPU-hours**
at today's rate. That is the number the whole effort exists to move.

---

## 1. Benchmark harness (build this first — analysis §"attack order" item 1)

Keep it to two layers. Resist a third.

**Layer A — end-to-end, `hyperfine` on the release binary.** No new code, no new dependency, and it
measures the thing users actually run (reject path + panel + emit, together). This is the primary
regression gate.

```sh
hyperfine --warmup 1 -L q requests,tensorflow,aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,rеquests \
  'target/release/sqdist --string {q} --list corpus/pypi-names.txt --typosquat'
```

**Layer B — kernel microbench, `criterion` in `benches/`.** Only for the DP/skeleton work in
analysis §1 and §3, where end-to-end noise (~5%) is larger than the effect. Add it when you get to
§1.3/§1.7/§1.8, not before. Note `panic = "abort"` in `[profile.release]` — cargo ignores it for the
bench profile, so criterion works, but don't be surprised by the warning.

**Allocation profile — `dhat`, once.** Analysis §2.4 is the one item flagged "profile needed to know
it exists". At n≈8 chars, mallocs plausibly dominate arithmetic. One `dhat` run on a 50k-name slice
answers it, and the answer decides whether §2.4 outranks §1.7. Do not skip this — it is the only
cheap way to avoid optimizing the DP kernel while the allocator eats the time.

**Skip:** a custom benchmark framework, per-axis timing instrumentation, a CI perf dashboard.
Add CI perf tracking only after a real regression slips through.

### Benchmark matrix (the axes that actually change behavior)

| Dimension | Values | Why |
|---|---|---|
| Query charset | ASCII / mixed-script homoglyph | Exercises the skeleton fast path and the `keyboard_distance` NA branch |
| Query length | 4 / 8 / 20 / 30 chars | DP is `O(nm)`; the prune wins scale with this |
| Mode | default panel / `-t 1` / `--typosquat` / `--typosquat --pypi` | Reject-path work differs per mode |
| Corpus size | 50k / 879k / 4.39M | Confirms linearity and sizes the index question (§5.1) |
| Hit density | `requests` (many near-misses) / a random UUID (near-zero hits) | Separates reject cost from emit cost |

---

## 2. What to measure, in what order

Ordered by (measured or certain payoff) ÷ (risk of changing output). Each item names its **stop
condition** — the measurement that says "done, move on".

### Tier 1 — certain wins, no profile needed, output-preserving

Bundle these into one change and measure once. Expect them to move the 3.0 s baseline meaningfully;
if they don't, that itself is the finding.

| Item | Analysis § | Expected |
|---|---|---|
| Identity short-circuit (`a == b` → constant panel) | 2.2 | Small on this corpus (1 hit / 879k), large on multi-query sweeps |
| Reuse `align`'s last cell as the Damerau distance (stop filling twice) | 1.1 | Large — one of two identical `O(nm)` fills, on every pair |
| `CONFUSABLES.to_vec()` → `Cow`/static borrow | 3.1 | Startup only; do it because it's free |
| `const MAX_KEY_DISTANCE` instead of O(K²) per pair | 4.1 | ~1100 `sqrt`s per pair — should be visible |
| O(1) ASCII `key_coord` table | 4.2 | See baseline fact 4 |
| ASCII fast path on `script_restriction` | 4.3 | Whole `unicode-security` walk skipped on ASCII pairs |
| Drop dead `ska`/`skb` `String`s | 2.4 | Two allocations per pair |
| `PreparedQuery` — precompute the query side once in list mode | 2.3 | Saves normalize + 2 char-collects + skeleton × 879k |

**Stop condition:** re-run the Layer A matrix. Whatever fraction remains tells you whether Tier 2 or
Tier 3 is next.

### Tier 2 — the reject path (analysis §2.1, §1.5, §1.6, §4.5)

Baseline fact 2 proves the panel runs on candidates that can never be emitted. Gate the panel:

```
combosquat check (cheap, string-only)
  → streaming skeleton equality (no String alloc, no DP)
  → |len(a) - len(b)| > k prune (free, exact: damerau ≥ |n-m|)
  → O(n) k=1 Damerau checker
  → full panel ONLY on rows that will be emitted
```

Under `--typosquat` on PyPI, a `requests` query should reject the overwhelming majority on the
length prune alone with zero DP. **This is very likely the single biggest win in the document.**

**Measurement:** count DP invocations before/after with a debug counter (temporary), then time it.
**Correctness gate:** §4 below — the output must be byte-identical.

**Watch out:** `is_possible_combosquat` must run *before* the length prune, or `lodash-utils` gets
dropped. Analysis §4.5 already flags this; it's the one place a "free" prune is not free.

### Tier 3 — allocation vs DP kernel (decided by the `dhat` run)

- If allocations dominate: scratch buffers, stack arrays for n ≤ 64, skeleton into `Vec<char>`.
- If DP dominates: prefix/suffix strip → `u8`/`u16` cells → 3-row OSA → peeled inner loop →
  `get_unchecked` → then and only then Myers/Hyyrö.

Do not start with Myers. It is the most code, the most risk, and the analysis explicitly warns it
can lose to a tiny stack DP at n≈8 — which is exactly the length distribution of registry names.

### Tier 4 — structural (only if list mode is the product)

Rayon over chunks is a handful of lines and should give near-linear scaling on an embarrassingly
parallel workload; try it before any index. A BK-tree / length-bucket index is a much bigger change
and only pays if per-pair cost is already low. Measure Tier 1–3 first — an index that saves 90% of
a cost you already cut 90% of is not worth its complexity.

---

## 3. Research spike: bulk package-name lists

All verified working on 2026-08-27. **Nothing here requires per-package iteration.**

### PyPI — recommended: the simple index (PEP 691 JSON)

```sh
curl -H 'Accept: application/vnd.pypi.simple.v1+json' https://pypi.org/simple/ -o pypi-simple.json
```

- **879,491 names, 43 MB, 0.58 s.** One request. This is the whole registry namespace.
- Names are already PEP 503 normalized — pairs nicely with `--pypi`.
- No auth, no billing, no API key. Etag/`x-pypi-last-serial` for cheap refresh.

**Verdict: use this.** deps.dev/BigQuery is strictly more setup for strictly less (see below).

### npm — recommended: `all-the-package-names`

```sh
curl -sSL https://unpkg.com/all-the-package-names/names.json -o npm-names.json
```

- **4,388,051 names, 114 MB, 8.7 s.** A plain JSON array, republished continuously.
- Alternative (authoritative but heavier): CouchDB replication at
  `https://replicate.npmjs.com/_all_docs`. Use only if you need provenance guarantees.

### deps.dev (what you asked about) — usable, but not the best fit here

`bigquery-public-data.deps_dev_v1` — 5 M packages / 50 M+ versions across npm, PyPI, Go, Maven,
Cargo. `PackageVersionsLatest` gives you `(System, Name, Version)`, so
`SELECT DISTINCT Name FROM … WHERE System='PYPI'` is a one-query name dump.

Trade-offs vs. the registry indexes:
- **Pro:** one uniform schema across five ecosystems; joins to `Advisories`, `Dependents`,
  `Projects` (i.e. popularity/criticality signals for free).
- **Con:** needs a GCP project; the free sandbox may not cover a full-table scan; snapshot-based, so
  it lags the live registry; **no non-BigQuery bulk export** — there is no "download the names file"
  URL.

**Verdict:** skip it for the name corpus, reach for it when you want *popularity-weighted* or
*cross-ecosystem* evaluation (§3 of the eval design below).

### Popularity / "what attackers target"

- **PyPI top 15,000 by download count:**
  `https://raw.githubusercontent.com/hugovk/top-pypi-packages/main/top-pypi-packages.json`
  (879 KB; `{project, download_count}` rows; verified: `boto3` #1 at 3.7 B downloads).
- **Cross-ecosystem counts and metadata:** [packages.ecosyste.ms](https://packages.ecosyste.ms) —
  free REST API, currently reports npm 5,791,250 / PyPI 931,090 packages. Good for sanity-checking
  corpus completeness; paginated, so not the fastest bulk path.

### Corpus sizes to expect

| Corpus | Names | Download | Time |
|---|---|---|---|
| PyPI simple index | 879,491 | 43 MB | 0.6 s |
| npm all-the-package-names | 4,388,051 | 114 MB | 8.7 s |
| PyPI top-15k | 15,000 | 0.9 MB | <1 s |

---

## 4. Research spike: known typosquat / combosquat incidents

Two distinct kinds of data, and the distinction matters a lot for evaluation.

### (a) Labeled attacker→victim **pairs** — the gold set

**[ecosyste-ms/typosquatting-dataset](https://github.com/ecosyste-ms/typosquatting-dataset)** — the
direct hit, and apparently the only curated public pair-labeled set.

```sh
curl -sSL https://raw.githubusercontent.com/ecosyste-ms/typosquatting-dataset/main/typosquats.csv
```

- **143 confirmed pairs**, 12 KB CSV, **CC0** (unrestricted).
- Schema: `malicious_package, target_package, ecosystem, registry, classification, source`.
- Ecosystems: PyPI 95, npm 35, Go 8, GitHub Actions 4, crates.io 1.
- **`classification` is the valuable column**: 13 attack techniques —
  replacement (28), omission (27), addition (24), transposition (22), repetition, … This maps almost
  one-to-one onto sqdist's axes, so you can report *recall per attack technique*, which is a far
  better evaluation than one aggregate number. Transposition in particular is precisely the
  Damerau-vs-Levenshtein distinction the tool exists to make.
- Aggregated from ossf/malicious-packages, DataDog, lxyeternal/pypi_malregistry, and academic papers.

**Caveat, and it's the important one: 143 pairs is small.** Enough for a correctness/recall gate,
too small for confident precision estimates or for benchmarking. Treat it as a unit-test corpus.

### (b) Malicious package **names** (unlabeled — no target attached)

| Source | Scale | Format / access |
|---|---|---|
| **[OSV](https://osv.dev) bulk zips** | **PyPI: 11,681 `MAL-` records** of 25,077 total; **npm: 220,150 `MAL-`** of 227,370 | `https://osv-vulnerabilities.storage.googleapis.com/{PyPI,npm}/all.zip` — 34 MB / 220 MB, one request, no auth |
| **[ossf/malicious-packages](https://github.com/ossf/malicious-packages)** | upstream of the above | OSV JSON in `osv/`, `withdrawn/` for retractions; Apache-2.0 |
| **[DataDog/malicious-software-packages-dataset](https://github.com/DataDog/malicious-software-packages-dataset)** | 28,623 packages, human-vetted | `samples/{pypi,npm,ai-skills,ide_extensions}/manifest.json` gives names without downloading the (password-`infected`) sample zips |
| **[lxyeternal/pypi_malregistry](https://github.com/lxyeternal/pypi_malregistry)** | 10,000+ PyPI | ASE 2023 paper dataset; ~750 MB; **no license declared — do not redistribute** |

**Recommended:** OSV `all.zip` per ecosystem. One curl, no auth, no rate limit, includes the DataDog
and OSSF contributions already, and `withdrawn/` gives you known false positives — which is a
genuinely useful negative-control set that most sources don't provide.

**The key structural insight:** OSV gives you ~232k malicious *names* but not their targets. Joining
those names against the top-15k popular list to *recover* the targets is **exactly the job sqdist
does**. So this is not just a benchmark corpus — it is the product's own task at full scale, with
the ecosyste.ms 143 pairs available as spot-check ground truth. That makes it the single best
evaluation workload in this document.

---

## 5. Evaluation design (correctness, not just speed)

Every optimization in §2 claims to preserve output. Prove it cheaply:

1. **Golden-output regression.** Before any change, capture
   `sqdist --string <q> --list pypi-names.txt` full JSONL for ~10 diverse queries. After each
   change, `diff`. Byte-identical or the change is wrong. This is one shell script and it makes the
   entire Tier 1/Tier 2 program safe to move fast on.
2. **Quality gate on the 143 labeled pairs.** Run each `malicious_package` as `--string` against a
   list containing its `target_package` plus the top-15k. Assert `--typosquat` classifies it
   `likely_typosquat` or `possible_combosquat`, and report recall **broken out by the
   `classification` column**. Any Tier 2 prune that drops recall on a technique is a bug in the
   prune, not a tuning question.
3. **Precision proxy at scale.** Sweep the top-15k queries against the full PyPI corpus and count
   emitted rows. Optimizations must not change the count. As a by-product this is the 14-CPU-hour
   workload from §0 — i.e. the thing that proves the speedup mattered.
4. **Non-ASCII coverage.** The corpora above are ASCII-dominated, so an ASCII fast path could regress
   the homoglyph path invisibly. Synthesize a homoglyph corpus by substituting Cyrillic/Greek
   look-alikes into the top-15k. This is the one corpus you have to generate rather than download.

---

## Suggested sequence

1. Download the four corpora (§3, §4a, §4b) into a git-ignored `corpus/`. ~160 MB, under a minute.
2. Write the golden-output diff script (§5.1) and the hyperfine matrix (§1). Two shell scripts.
3. One `dhat` run on a 50k slice — it decides Tier 3's ordering.
4. Tier 1 as a single change. Measure.
5. Tier 2 reject-path gating. Measure. Expect this to be the headline number.
6. Re-profile. Only then decide whether Tier 3's kernel work or Tier 4's rayon/index is next.

Corpus data is not committed — it's large, it changes, and every source above is a single
unauthenticated `curl`. A `scripts/fetch-corpus.sh` is the right footprint.
