//! Regression for issue #2356 — God-Pharaoh's Gift.
//!
//! "At the beginning of combat on your turn, you may exile a creature card from
//!  your graveyard. If you do, create a token that's a copy of that card,
//!  except it's a 4/4 black Zombie. It gains haste until end of turn."
//!
//! The trailing "It gains haste" sentence is a bare-pronoun anaphor that refers
//! to the just-created token (CR 608.2k), not to the source artifact. The
//! parser previously resolved the bare "it" to `TargetFilter::SelfRef`, so the
//! haste was granted to God-Pharaoh's Gift itself and the token never gained
//! haste. The fix marks the parse context once a token-creating effect
//! (`Token` / `CopyTokenOf` / `Populate`) has been parsed in the chain, so the
//! pronoun resolvers bind a following "it"/"they" to `LastCreated`.

use engine::parser::parse_oracle_text;
use serde_json::Value;

/// The `affected` filter type of every static ability that grants Haste in the
/// serialized parse — this is where a "[subject] gains haste" continuous
/// modification records its subject.
fn haste_grant_targets(parsed: &impl serde::Serialize) -> Vec<String> {
    let json = serde_json::to_value(parsed).expect("serialize");
    let mut out = Vec::new();
    collect(&json, &mut out);
    out
}

fn grants_haste(modifications: &Value) -> bool {
    let s = modifications.to_string();
    s.contains("\"AddKeyword\"") && s.contains("\"Haste\"")
}

fn collect(v: &Value, out: &mut Vec<String>) {
    if let Value::Object(map) = v {
        if let (Some(affected), Some(mods)) = (map.get("affected"), map.get("modifications")) {
            if grants_haste(mods) {
                let target = affected
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("<none>")
                    .to_string();
                out.push(target);
            }
        }
    }
    match v {
        Value::Object(map) => map.values().for_each(|c| collect(c, out)),
        Value::Array(arr) => arr.iter().for_each(|c| collect(c, out)),
        _ => {}
    }
}

#[test]
fn gpg_trailing_it_gains_haste_binds_to_created_token() {
    const GPG: &str = "At the beginning of combat on your turn, you may exile a creature card from your graveyard. If you do, create a token that's a copy of that card, except it's a 4/4 black Zombie. It gains haste until end of turn.";
    let parsed = parse_oracle_text(
        GPG,
        "God-Pharaoh's Gift",
        &[],
        &["Artifact".to_string()],
        &[],
    );
    let targets = haste_grant_targets(&parsed);
    assert!(
        !targets.is_empty(),
        "expected a Haste grant in the parsed effect chain"
    );
    assert!(
        targets.iter().all(|t| t == "LastCreated"),
        "the token's Haste grant must bind to LastCreated, got {targets:?}"
    );
}

#[test]
fn plural_they_gain_haste_binds_to_created_tokens() {
    const TEXT: &str = "When this creature dies, create two 1/1 red Goblin creature tokens. They gain haste until end of turn.";
    let parsed = parse_oracle_text(TEXT, "Goblin Maker", &[], &["Creature".to_string()], &[]);
    let targets = haste_grant_targets(&parsed);
    assert!(
        !targets.is_empty(),
        "expected a Haste grant in the parsed effect chain"
    );
    assert!(
        targets.iter().all(|t| t == "LastCreated"),
        "the tokens' Haste grant must bind to LastCreated, got {targets:?}"
    );
}

#[test]
fn self_haste_without_token_creation_still_binds_self() {
    // Regression guard: with no prior token-creating effect, a bare "it gains
    // haste" must keep binding to the source (SelfRef), never LastCreated.
    const TEXT: &str = "When this creature enters, it gains haste until end of turn.";
    let parsed = parse_oracle_text(TEXT, "Hasty One", &[], &["Creature".to_string()], &[]);
    let targets = haste_grant_targets(&parsed);
    assert!(
        !targets.is_empty(),
        "expected a Haste grant in the parsed effect chain"
    );
    assert!(
        targets.iter().all(|t| t == "SelfRef"),
        "self haste grant must bind to SelfRef, got {targets:?}"
    );
}
