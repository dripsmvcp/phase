//! Regression coverage for Ulamog, the Defiler (issue #2353).
//!
//! Ulamog's Oracle text composes four shipped building blocks:
//!   1. ETB counters (THE REPORTED BUG): "Ulamog enters with a number of +1/+1
//!      counters on it equal to the greatest mana value among cards in exile."
//!      → `Effect::PutCounter { count: QuantityExpr::Ref { Aggregate { Max,
//!      ManaValue, <cards in exile> } } }` carried by a self ETB replacement
//!      (CR 614.1c). At resolution the dynamic count resolves against the exile
//!      zone (CR 202.3 mana value; CR 608.2h present-tense aggregate).
//!   2. Dynamic Annihilator: "Ulamog has annihilator X, where X is the number of
//!      +1/+1 counters on it." → a self CDA static carrying
//!      `ContinuousModification::AddDynamicKeyword { Annihilator, CountersOn {
//!      Source, P1P1 } }`. After layers (CR 613), the object surfaces
//!      `Keyword::Annihilator(N)` where N = its +1/+1 counter count
//!      (CR 702.86a).
//!   3. Ward—Sacrifice two permanents (CR 702.21): `Keyword::Ward(Sacrifice {
//!      count: 2, Permanent })`.
//!   4. Cast trigger (CR 603.2): "target opponent exiles the top half of their
//!      library, rounded up." → `Effect::ExileTop` with a half-library rounded-up
//!      count (CR 107.3 rounding).
//!
//! The reported bug was that Ulamog entered with ZERO +1/+1 counters. The
//! runtime test below is DISCRIMINATING: it stages cards of various mana values
//! in exile and asserts the entering Ulamog carries exactly `greatest MV`
//! counters — and that the dynamic Annihilator reflects that count.

use engine::game::scenario::GameScenario;
use engine::game::zones::create_object;
use engine::types::ability::{
    AggregateFunction, ContinuousModification, Effect, ObjectProperty, QuantityExpr, QuantityRef,
    TargetFilter,
};
use engine::types::card_type::CoreType;
use engine::types::counter::CounterType;
use engine::types::identifiers::{CardId, ObjectId};
use engine::types::keywords::{DynamicKeywordKind, Keyword, WardCost};
use engine::types::mana::{ManaCost, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);

/// Ulamog, the Defiler — verified exact Oracle text from Scryfall
/// (`https://api.scryfall.com/cards/named?exact=Ulamog,+the+Defiler`).
const ULAMOG_ORACLE: &str = "When you cast this spell, target opponent exiles the top half of their library, rounded up.\nWard—Sacrifice two permanents.\nUlamog enters with a number of +1/+1 counters on it equal to the greatest mana value among cards in exile.\nUlamog has annihilator X, where X is the number of +1/+1 counters on it.";

/// Place a card with mana value `cmc` into the shared Exile zone. Mirrors the
/// `add_library_creature_with_cmc` helper in `chord_of_calling.rs`:
/// `ManaCost::generic(cmc)` fixes the mana value (CR 202.3).
fn add_exile_card_with_cmc(
    runner: &mut engine::game::scenario::GameRunner,
    name: &str,
    cmc: u32,
) -> ObjectId {
    let card_id = CardId(runner.state().next_object_id);
    let id = create_object(
        runner.state_mut(),
        card_id,
        P1,
        name.to_string(),
        Zone::Exile,
    );
    let obj = runner.state_mut().objects.get_mut(&id).unwrap();
    obj.card_types.core_types.push(CoreType::Creature);
    obj.mana_cost = ManaCost::generic(cmc);
    id
}

/// Add `count` units of generic-payable mana to P0's pool (colorless suffices for
/// Ulamog's all-generic {10} cost).
fn add_mana(runner: &mut engine::game::scenario::GameRunner, count: usize) {
    for _ in 0..count {
        let unit = ManaUnit::new(ManaType::Colorless, ObjectId(0), false, vec![]);
        runner.state_mut().players[0].mana_pool.add(unit);
    }
}

// ---------------------------------------------------------------------------
// Test 1 (SHAPE): Ulamog's full Oracle parses into all four building blocks.
// ---------------------------------------------------------------------------

/// SHAPE — assert the parsed AST shape for all four Ulamog components. Labelled
/// SHAPE per the runtime-tests-must-drive-the-pipeline rule; it asserts static
/// parse output only via semantic accessors, not a driven transition.
#[test]
fn ulamog_defiler_full_oracle_parses_all_components() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let builder =
        scenario.add_creature_to_hand_from_oracle(P0, "Ulamog, the Defiler", 7, 7, ULAMOG_ORACLE);
    let id = builder.id();
    let runner = scenario.build();
    let obj = &runner.state().objects[&id];

    // (1) THE BUG — ETB +1/+1 counters with a DYNAMIC greatest-MV-in-exile count.
    // Carried as a self ETB replacement (CR 614.1c).
    let etb_count = obj
        .replacement_definitions
        .as_slice()
        .iter()
        .find_map(|r| {
            r.execute.as_ref().and_then(|ab| match &*ab.effect {
                Effect::PutCounter {
                    counter_type,
                    count,
                    ..
                } if *counter_type == CounterType::Plus1Plus1 => Some(count.clone()),
                _ => None,
            })
        })
        .unwrap_or_else(|| {
            panic!(
                "Ulamog must carry a self ETB PutCounter(+1/+1) replacement, got {:?}",
                obj.replacement_definitions.as_slice()
            )
        });
    // CR 202.3 + CR 608.2h: the count must be the dynamic greatest mana value
    // aggregate over cards in exile — NOT a fixed value (the bug = Fixed(0)).
    match etb_count {
        QuantityExpr::Ref {
            qty:
                QuantityRef::Aggregate {
                    function: AggregateFunction::Max,
                    property: ObjectProperty::ManaValue,
                    ref filter,
                },
        } => {
            assert!(
                !matches!(filter, TargetFilter::Any),
                "ETB counter aggregate must scope to cards in exile, got Any"
            );
        }
        other => {
            panic!("ETB counter count must be Aggregate(Max, ManaValue, <exile>), got {other:?}")
        }
    }

    // (2) Dynamic Annihilator X = number of +1/+1 counters on it (CR 702.86a).
    let has_dynamic_annihilator = obj.static_definitions.as_slice().iter().any(|s| {
        s.modifications.iter().any(|m| {
            matches!(
                m,
                ContinuousModification::AddDynamicKeyword {
                    kind: DynamicKeywordKind::Annihilator,
                    value: QuantityExpr::Ref {
                        qty: QuantityRef::CountersOn {
                            counter_type: Some(CounterType::Plus1Plus1),
                            ..
                        },
                    },
                }
            )
        })
    });
    assert!(
        has_dynamic_annihilator,
        "Ulamog must carry a dynamic Annihilator(counters-on-self) static, got {:?}",
        obj.static_definitions
    );

    // (3) Ward—Sacrifice two permanents (CR 702.21).
    assert!(
        obj.keywords
            .iter()
            .any(|k| matches!(k, Keyword::Ward(WardCost::Sacrifice { count: 2, .. }))),
        "Ulamog must carry Ward(Sacrifice {{ count: 2 }}), got {:?}",
        obj.keywords
    );

    // (4) Cast trigger: target opponent exiles top half of library, rounded up.
    let has_cast_exile_trigger = obj.trigger_definitions.as_slice().iter().any(|t| {
        matches!(
            t.execute.as_deref().map(|ab| &*ab.effect),
            Some(Effect::ExileTop { .. })
        )
    });
    assert!(
        has_cast_exile_trigger,
        "Ulamog must carry a cast-triggered ExileTop, got {:?}",
        obj.trigger_definitions.as_slice()
    );
}

// ---------------------------------------------------------------------------
// Test 2 (RUNTIME): the discriminating full-pipeline test for THE BUG.
// ---------------------------------------------------------------------------

/// RUNTIME — CR 614.1c + CR 202.3 + CR 702.86a. Stage cards of MV 3, 7, and 5 in
/// exile, cast Ulamog through the full pipeline, and assert the entering Ulamog
/// carries exactly 7 (= greatest MV among cards in exile) +1/+1 counters — and
/// that the dynamic Annihilator surfaces as `Annihilator(7)` after layers.
///
/// This is DISCRIMINATING: before the fix Ulamog entered with 0 counters, so the
/// `assert_eq!(counters, 7)` fails on the buggy behavior.
#[test]
fn ulamog_defiler_enters_with_greatest_exile_mv_counters() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mut builder =
        scenario.add_creature_to_hand_from_oracle(P0, "Ulamog, the Defiler", 7, 7, ULAMOG_ORACLE);
    // CR 202.1: Ulamog's mana cost is {10}. `from_oracle_text` preserves
    // mana_cost; set it explicitly so the pipeline's cost is well-defined.
    builder.with_mana_cost(ManaCost::generic(10));
    let ulamog = builder.id();

    let mut runner = scenario.build();

    // Stage cards in exile with mana values 3, 7, 5. Greatest = 7.
    add_exile_card_with_cmc(&mut runner, "Exiled MV3", 3);
    add_exile_card_with_cmc(&mut runner, "Exiled MV7", 7);
    add_exile_card_with_cmc(&mut runner, "Exiled MV5", 5);

    // Give P1 a small library so the cast trigger's "exile top half" has a target
    // (it must not interfere with the ETB-counter assertion).
    for i in 0..4 {
        let card_id = CardId(runner.state().next_object_id);
        let id = create_object(
            runner.state_mut(),
            card_id,
            P1,
            format!("P1 Lib {i}"),
            Zone::Library,
        );
        runner
            .state_mut()
            .objects
            .get_mut(&id)
            .unwrap()
            .card_types
            .core_types
            .push(CoreType::Creature);
    }

    // Pay {10} from pool.
    add_mana(&mut runner, 10);

    // CR 601.2: cast Ulamog. The cast trigger targets P1 (the opponent); the
    // driver answers that target slot via `.target_player(P1)`. The pool
    // auto-pays {10}; resolution puts Ulamog onto the battlefield, where the
    // self ETB-counter replacement applies (CR 614.1c).
    runner.cast(ulamog).target_player(P1).resolve();
    runner.advance_until_stack_empty();

    // CR 202.3 + CR 608.2h: greatest MV among the exiled cards is 7.
    let counters = runner.state().objects[&ulamog]
        .counters
        .get(&CounterType::Plus1Plus1)
        .copied()
        .unwrap_or(0);
    assert_eq!(
        counters, 7,
        "Ulamog must enter with +1/+1 counters = greatest MV among cards in exile (7), \
         got {counters} (the reported bug was 0)"
    );

    // CR 702.86a + CR 613: the dynamic Annihilator surfaces the counter count.
    // After layers, the object carries `Keyword::Annihilator(7)`.
    let obj = &runner.state().objects[&ulamog];
    assert!(
        obj.keywords.contains(&Keyword::Annihilator(7)),
        "Ulamog must surface Annihilator(7) (= its +1/+1 counter count), got keywords {:?}",
        obj.keywords
    );
}
