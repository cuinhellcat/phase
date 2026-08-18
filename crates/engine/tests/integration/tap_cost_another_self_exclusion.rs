//! CR 602.2b + CR 118.3: a `"Tap another untapped … you control"` activation
//! cost excludes the ability's own source (#7522).
//!
//! Spire Mechcycle's exhaust cost reads "Tap another untapped Mount or Vehicle
//! you control", and the Mechcycle is itself a Vehicle — before the fix the
//! parsed cost filter carried no `FilterProp::Another`, so the source was an
//! eligible payment for its own ability.
//!
//! The runtime was never the defect. `has_enough_tap_creatures`
//! (`game/cost_payability.rs`) evaluates the cost filter against a
//! `FilterContext::from_source`, so `FilterProp::Another` is honoured the
//! moment the parser emits it; its separate `exclude_source` flag belongs to
//! composite `{T}` costs and is untouched here. These tests therefore drive the
//! real payability gate (`ai_support::legal_actions`), not the parser.
//!
//! Card text is built from Oracle text rather than named cards, so the tests
//! run in CI (which has no card database).
//!
//! Not covered: "other than this creature" tail forms (Impelled Giant,
//! Mossbridge Troll) reach the exclusion through a different grammar and were
//! already correct; the subtype disjunction (Spire Mechcycle's "Mount or
//! Vehicle") is pinned at the parser layer in
//! `parser::oracle_cost::tests::tap_cost_another_marks_every_leg_of_a_disjunction`,
//! because `GameScenario` has no helper that stamps a Vehicle subtype.

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;

/// The reported shape: the cost excludes the source.
const ANOTHER: &str =
    "Tap another untapped creature you control: This creature gains indestructible until end of turn.";

/// CR 601.2b counter-direction: no "another", so the source IS eligible.
const PLAIN: &str =
    "Tap an untapped creature you control: This creature gains indestructible until end of turn.";

/// One creature carrying `oracle` plus `helpers` vanilla untapped creatures,
/// all controlled by P0, with P0 holding priority in its precombat main phase.
fn board(oracle: &str, helper_count: usize) -> (GameRunner, ObjectId, Vec<ObjectId>) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let source = scenario
        .add_creature_from_oracle(P0, "Tapper", 2, 2, oracle)
        .id();
    let helpers = (0..helper_count)
        .map(|i| scenario.add_creature(P0, &format!("Helper {i}"), 1, 1).id())
        .collect();
    (scenario.build(), source, helpers)
}

/// Does the engine offer `source`'s first activated ability right now?
fn offers_activation(runner: &GameRunner, source: ObjectId) -> bool {
    engine::ai_support::legal_actions(runner.state())
        .iter()
        .any(|action| {
            matches!(
                action,
                GameAction::ActivateAbility {
                    source_id,
                    ability_index: 0,
                } if *source_id == source
            )
        })
}

/// The defect: alone on the battlefield, the source matched its own cost filter
/// and the ability was offered — it would have paid by tapping itself.
///
/// The negative assertion is not vacuous: `a_plain_tap_cost_still_includes_the_source`
/// runs the IDENTICAL board with the article form and finds the ability offered,
/// so an unreachable-ability setup would fail there.
///
/// Reverting the fix flips this test to red: `assert!(!offers_activation(…))`
/// fails, and the `Err` expectation below becomes `Ok`.
#[test]
fn the_source_alone_cannot_pay_its_own_tap_another_cost() {
    let (mut runner, source, _) = board(ANOTHER, 0);
    assert!(
        !offers_activation(&runner, source),
        "a lone source must not be offered its own \"tap another untapped creature\" ability"
    );
    assert!(
        runner
            .act(GameAction::ActivateAbility {
                source_id: source,
                ability_index: 0,
            })
            .is_err(),
        "activating anyway must be rejected — the source may not tap itself for the cost"
    );
}

/// Positive counter-direction: with a second untapped creature the cost is
/// payable and the ability is offered. Guards against over-suppression, the
/// expensive collateral of a self-exclusion fix.
#[test]
fn a_second_untapped_creature_pays_the_tap_another_cost() {
    let (mut runner, source, helpers) = board(ANOTHER, 1);
    assert!(
        offers_activation(&runner, source),
        "another untapped creature makes the cost payable"
    );
    let helper = helpers[0];
    runner.activate(source, 0).pay_with(&[helper]).resolve();
    assert!(
        runner.state().objects[&helper].tapped,
        "the selected helper must be tapped by the real activation cost payment"
    );
    assert!(
        !runner.state().objects[&source].tapped,
        "the source must remain untapped; only another creature pays this cost"
    );
}

/// CR 601.2b: a standalone `TapCreatures` cost with no "another" DOES include
/// the source. This is the behaviour the fix must not break — and it is the
/// reach guard for the negative assertion in the first test.
#[test]
fn a_plain_tap_cost_still_includes_the_source() {
    let (runner, source, _) = board(PLAIN, 0);
    assert!(
        offers_activation(&runner, source),
        "\"tap an untapped creature you control\" is payable by the source itself (CR 601.2b)"
    );
}
