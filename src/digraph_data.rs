//! Curated digraph -> confusable mappings (non-UTS#39, opt-in via
//! `--confusables=digraph`). Seeded by dnstwist's glyphs_ascii (Apache-2.0); this
//! short curated list is our own. One-way: the digraph impersonates the target.
//!
//! Anchoring to UTS#39: replacements are the UTS#39 *skeleton* of the impersonated
//! char, not the char itself, so the digraph unifies with that char under the
//! single-pass `skeleton()`. Concretely, a "looks like m" digraph maps to m's
//! UTS#39 skeleton "rn" (NOT "m"), because UTS#39 canonicalizes m -> "rn"; mapping
//! nn -> "m" would leave nn ("m") and m ("rn") un-unified. So we take vv->w, cl->d,
//! and nn (anchored to "rn"). rn is intentionally NOT in this table: UTS#39 already
//! makes rn <-> m confusable via m -> "rn", and adding an inverse rn -> "m" digraph
//! would break that (skeleton("rnicrosoft") would stop equalling skeleton("microsoft")).
//!
//! FP note for future maintainers (mitigations NOT built — the opt-in is the
//! consent): cl->d is the highest-FP rule (clear->dear, clock->dock). If FP
//! complaints arise, the menu is: same-script-Latin gating; a short-name /
//! font-context gate (nn->rn is a small-size proportional-font merging effect); a
//! per-pair deny list; tiny-edit-distance gating. See the research/design docs.
pub static DIGRAPHS: &[(&str, &str)] = &[("vv", "w"), ("cl", "d"), ("nn", "rn")];
