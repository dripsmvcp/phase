//! Regression for issue #4238 — Altar of the Brood is not triggered when
//! Incubate creates an Incubator token.
//!
//! Altar of the Brood: "Whenever another permanent you control enters, each
//! opponent mills a card."
//!
//! Incubate creates an Incubator token that enters the battlefield under your
//! control. That entry is a zone change from outside the game (CR 111.1 /
//! CR 603.6a), so it must emit an ETB event that "another permanent you control
//! enters" triggers observe. The bug: `incubate::resolve` placed the token via
//! `zones::create_object` and emitted no ETB `ZoneChanged`/`TokenCreated`, so
//! Altar (and every other entering-permanent trigger) never fired for the
//! Incubator.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::card_type::CoreType;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;

const ALTAR: &str = "Whenever another permanent you control enters, each opponent mills a card.";

fn library_count(
    runner: &engine::game::scenario::GameRunner,
    player: engine::types::player::PlayerId,
) -> usize {
    runner
        .state()
        .players
        .iter()
        .find(|p| p.id == player)
        .map(|p| p.library.len())
        .expect("player exists")
}

fn graveyard_count(
    runner: &engine::game::scenario::GameRunner,
    player: engine::types::player::PlayerId,
) -> usize {
    runner
        .state()
        .players
        .iter()
        .find(|p| p.id == player)
        .map(|p| p.graveyard.len())
        .expect("player exists")
}

#[test]
fn incubate_token_entry_triggers_altar_of_the_brood() {
    let mut scenario = GameScenario::new_n_player(2, 42);
    scenario.at_phase(Phase::PreCombatMain);

    // The opponent needs a library to mill from.
    for _ in 0..10 {
        scenario.add_card_to_library_top(P1, "Lib Card");
    }

    // Altar of the Brood on P0's battlefield, parsed from its real Oracle text.
    // 1/1 so it survives state-based actions and keeps its trigger active.
    scenario.add_creature_from_oracle(P0, "Altar of the Brood", 1, 1, ALTAR);

    // A 0-cost sorcery that just incubates 1 — when it resolves the Incubator
    // token enters under P0's control, which must trigger Altar.
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Test Incubate", false, "Incubate 1.")
        .with_mana_cost(ManaCost::zero())
        .id();

    let mut runner = scenario.build();
    let spell_card = runner.state().objects[&spell].card_id;

    let lib_before = library_count(&runner, P1);
    let gy_before = graveyard_count(&runner, P1);

    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id: spell_card,
            targets: vec![],
        })
        .expect("casting a 0-cost incubate sorcery must succeed");
    runner.advance_until_stack_empty();

    // The Incubator token entered under P0's control.
    let incubators = runner
        .state()
        .battlefield
        .iter()
        .filter_map(|id| runner.state().objects.get(id))
        .filter(|o| {
            o.controller == P0
                && o.card_types.core_types.contains(&CoreType::Artifact)
                && o.card_types.subtypes.iter().any(|s| s == "Incubator")
        })
        .count();
    assert_eq!(
        incubators, 1,
        "Incubate must create one Incubator token under P0"
    );

    // Altar saw the Incubator enter and milled the opponent exactly once.
    assert_eq!(
        lib_before - library_count(&runner, P1),
        1,
        "Altar of the Brood must mill the opponent when the Incubator enters"
    );
    assert_eq!(
        graveyard_count(&runner, P1) - gy_before,
        1,
        "the milled card lands in the opponent's graveyard"
    );
}
