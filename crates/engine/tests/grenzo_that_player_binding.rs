//! Regression test for #2346 — Grenzo, Havoc Raiser's combat-damage modal
//! trigger bound "that player" to the ability's controller (the player who
//! dealt the damage) instead of the damaged player.
//!
//! Oracle text (verified from card-data.json):
//!   "Whenever a creature you control deals combat damage to a player, choose
//!    one —
//!    • Goad target creature that player controls.
//!    • Exile the top card of that player's library. Until end of turn, you may
//!      cast that card and you may spend mana as though it were mana of any
//!      color to cast that spell."
//!
//! Two parser defects made both modes affect the wrong player:
//!   1. `lower_mode_abilities_with_subject` parsed each `Choose one —` mode body
//!      in a fresh `ParseContext` that dropped the trigger's relative-player
//!      scope, so "that player" fell back to the controller (ExileTop emitted
//!      `ParentTarget`).
//!   2. The Goad arm of `parse_imperative_family_ast` parsed its target with the
//!      context-free `parse_target`, so "that player controls" resolved to
//!      `ControllerRef::You` even when the surrounding scope was set.
//!
//! CR 603.7c + CR 120.3: "deals combat damage to a player" is a DamageDone
//! trigger whose damaged player is bound as `TriggeringPlayer` in the event
//! context. The fix threads the trigger's relative-player scope into modal mode
//! bodies and routes Goad through the context-aware target parser, so both modal
//! and non-modal Goad correctly bind "that player controls" to TriggeringPlayer.

use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::{ControllerRef, Effect, TargetFilter};

const GRENZO_ORACLE: &str = "Whenever a creature you control deals combat damage to a player, choose one —\n\
• Goad target creature that player controls.\n\
• Exile the top card of that player's library. Until end of turn, you may cast that card and you may spend mana as though it were mana of any color to cast that spell.";

/// Read the `controller` of a `TargetFilter::Typed` target, if present.
fn typed_controller(filter: &TargetFilter) -> Option<&ControllerRef> {
    match filter {
        TargetFilter::Typed(typed) => typed.controller.as_ref(),
        _ => None,
    }
}

#[test]
fn grenzo_modal_binds_that_player_to_the_damaged_player() {
    let parsed = parse_oracle_text(
        GRENZO_ORACLE,
        "Grenzo, Havoc Raiser",
        &[],
        &["Creature".to_string()],
        &["Goblin".to_string()],
    );

    let trigger = parsed
        .triggers
        .iter()
        .find(|t| {
            t.execute
                .as_ref()
                .is_some_and(|e| !e.mode_abilities.is_empty())
        })
        .expect("Grenzo's combat-damage trigger must parse as a modal ability");
    let modes = &trigger.execute.as_ref().unwrap().mode_abilities;
    assert_eq!(modes.len(), 2, "Grenzo is a two-mode `choose one —`");

    // Mode 0: "Goad target creature that player controls." The creature filter's
    // controller must be the damaged player (TriggeringPlayer per CR 603.7c —
    // DamageDone triggers bind the damaged player as TriggeringPlayer), not You.
    match modes[0].effect.as_ref() {
        Effect::Goad { target } => assert_eq!(
            typed_controller(target),
            Some(&ControllerRef::TriggeringPlayer),
            "CR 109.4 + CR 603.7c: Goad must target a creature the damaged player controls \
             (TriggeringPlayer), not the ability's controller. got: {target:?}"
        ),
        other => panic!("mode 0 must be Goad, got {other:?}"),
    }

    // Mode 1: "Exile the top card of that player's library." The library owner
    // must be the damaged player (event-bound TriggeringPlayer), not ParentTarget.
    match modes[1].effect.as_ref() {
        Effect::ExileTop { player, .. } => assert_eq!(
            player,
            &TargetFilter::TriggeringPlayer,
            "CR 120.3 + CR 608.2i: ExileTop must read the damaged player's \
             library (TriggeringPlayer), not the controller. got: {player:?}"
        ),
        other => panic!("mode 1 must be ExileTop, got {other:?}"),
    }
}

#[test]
fn non_modal_goad_honors_relative_player_scope() {
    // Building-block control: the Goad target-parsing fix applies outside the
    // modal path too — a single-mode combat-damage trigger binds "that player
    // controls" to the damaged player exactly like the modal mode body.
    let parsed = parse_oracle_text(
        "Whenever a creature you control deals combat damage to a player, goad target creature that player controls.",
        "Test Goader",
        &[],
        &["Creature".to_string()],
        &[],
    );
    let trigger = parsed
        .triggers
        .iter()
        .find_map(|t| t.execute.clone())
        .expect("combat-damage trigger must parse a body");
    match trigger.effect.as_ref() {
        Effect::Goad { target } => assert_eq!(
            typed_controller(target),
            Some(&ControllerRef::TriggeringPlayer),
            "CR 603.7c: non-modal Goad must bind to the damaged player (TriggeringPlayer), got: {target:?}"
        ),
        other => panic!("expected Goad, got {other:?}"),
    }
}
