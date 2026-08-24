//! CR 118.9 + CR 601.2a (#7575): "Once each turn, you may pay {0} rather than
//! pay the mana cost for a spell you cast from exile." (Warped Space) lowered
//! its whole line to `Effect::Unimplemented`: `parse_spells_alternative_cost`
//! strict-failed on the origin-zone qualifier after "spell you cast".
//!
//! Class (measured against card-data.json, 4 cards, all Unimplemented before
//! the fix): Warped Space, Dragon's Smile, Tlincalli Hunter ("from exile"),
//! Darksteel Monolith ("a colorless spell you cast from your hand").
//!
//! The runtime needed no change: `granted_spell_alternative_cost` evaluates the
//! `affected` filter through the spell-filter path, whose
//! `FilterProp::InZone` arm compares the cast's ORIGIN zone. These tests host
//! the static on a plain enchantment — Room door gating is #7573's merged
//! concern, not this one.

use engine::game::scenario::{GameScenario, P0};
use engine::types::ability::{CastingPermission, Duration};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::statics::CastFrequency;
use engine::types::zones::{EtbTapState, Zone};

const WARPED_SPACE_TEXT: &str = "Once each turn, you may pay {0} rather than pay the mana cost for a spell you cast from exile.";

fn exile_permission() -> CastingPermission {
    CastingPermission::PlayFromExile {
        duration: Duration::UntilEndOfTurn,
        granted_to: P0,
        frequency: CastFrequency::Unlimited,
        source_id: None,
        invalidation: None,
        exiled_by_ability_controller: None,
        mana_spend_permission: None,
        card_filter: None,
        single_use_group: None,
        single_use: false,
        cast_cost_raise: None,
        land_enter_tapped: EtbTapState::Unspecified,
    }
}

fn cast(runner: &mut engine::game::scenario::GameRunner, spell: ObjectId) -> bool {
    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .is_ok()
}

/// CR 118.9 + CR 601.2b: the first exile cast each turn may pay {0} — with an
/// EMPTY pool and an unpayable printed {4}, the cast must succeed through the
/// grant. The second exile cast that turn gets no choice and dies unpaid.
///
/// Discriminating: before the fix the line is Unimplemented, no grant exists,
/// and the first cast is rejected outright.
#[test]
fn warped_space_lets_one_exile_cast_pay_zero_each_turn() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_enchantment_from_oracle(P0, "Warped Static Host", WARPED_SPACE_TEXT);
    let first = scenario
        .add_creature_to_exile(P0, "First Exile", 2, 2)
        .with_mana_cost(ManaCost::generic(4))
        .id();
    let second = scenario
        .add_creature_to_exile(P0, "Second Exile", 2, 2)
        .with_mana_cost(ManaCost::generic(4))
        .id();
    let mut runner = scenario.build();
    for id in [first, second] {
        runner
            .state_mut()
            .objects
            .get_mut(&id)
            .unwrap()
            .casting_permissions
            .push(exile_permission());
    }

    assert!(
        cast(&mut runner, first),
        "the {{0}} grant must admit the cast with an empty pool"
    );
    match &runner.state().waiting_for {
        WaitingFor::OptionalCostChoice { .. } => {}
        other => panic!("expected the {{0}}-vs-printed OptionalCostChoice, got {other:?}"),
    }
    runner
        .act(GameAction::DecideOptionalCost { pay: true })
        .expect("accepting the {0} alternative must succeed");
    assert_eq!(
        runner.state().objects[&first].zone,
        Zone::Stack,
        "the spell must reach the stack having paid {{0}}"
    );

    // CR 302.1: creature spells need an empty stack; resolve before the probe.
    runner.resolve_top();

    // CR 118.9 + CR 601.2b: "Once each turn" — the slot is spent, the second
    // exile cast gets no {0} choice, and with an empty pool it must die.
    assert!(
        !cast(&mut runner, second),
        "the second exile cast this turn must not get the spent {{0}} slot"
    );
}

/// PIN (green before the fix too, Regel 5): the grant is scoped to casts FROM
/// EXILE — a hand cast with an empty pool must stay rejected while the grant
/// is active. Guards the InZone prop against being dropped or over-broadened.
#[test]
fn warped_space_does_not_reach_a_hand_cast() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_enchantment_from_oracle(P0, "Warped Static Host", WARPED_SPACE_TEXT);
    let hand = scenario
        .add_creature_to_hand(P0, "Hand Bear", 2, 2)
        .with_mana_cost(ManaCost::generic(4))
        .id();
    let mut runner = scenario.build();

    assert!(
        !cast(&mut runner, hand),
        "an exile-scoped grant must not admit a hand cast with an empty pool"
    );
}
