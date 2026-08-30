//! CR 702.26f: a "for as long as ~ remains on the battlefield" steal ends when
//! the stealing permanent phases out — and does NOT come back when it phases
//! back in — while the CR 702.26d event-deadline class ("until ~ leaves the
//! battlefield", Banisher Priest's exile) keeps running across the same
//! phase-out.
//!
//! Sower of Temptation's steal lowers to a `WhileHostOnBattlefield` transient
//! continuous effect. Before the wording split the duration lowered to the
//! `UntilHostLeavesPlay` event deadline, `transient_effect_is_live` asked only
//! the zone question, and a phased-out Sower kept its stolen creature —
//! against CR 702.26f: "effects with 'for as long as' durations that track
//! that permanent (see rule 611.2b) end when that permanent phases out because
//! they can no longer see it."
//!
//! Both tests are discriminating, in opposite directions:
//!   * Revert the parser split (map "remains on the battlefield" back onto
//!     `UntilHostLeavesPlay`) or drop the presence arm of
//!     `prune_lapsed_host_bound_effects` → the steal survives the phase-out →
//!     the first test fails.
//!   * Ask the phasing question of EVERY host-bound duration (the over-reach a
//!     previous review round shipped and reverted) → Banisher Priest's exile
//!     would end at the phase-out and the exiled creature return early → the
//!     second test fails.
//!
//! The phase-out goes through the production entry point,
//! `game::phasing::phase_out_object` — the same call
//! `effects::phase_out::resolve` makes for Clever Concealment and Teferi's
//! Protection — so the test proves the engine's own phase-out reaches the
//! pruning seam, not merely that a predicate reads a hand-set field.

use engine::game::game_object::PhaseOutCause;
use engine::game::layers::evaluate_layers;
use engine::game::phasing::{phase_in_object, phase_out_object};
use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const SOWER_ORACLE: &str = "Flying\nWhen this creature enters, gain control of \
    target creature for as long as this creature remains on the battlefield.";

const BANISHER_PRIEST_ORACLE: &str = "When this creature enters, exile target \
    creature an opponent controls until this creature leaves the battlefield.";

#[test]
fn sowers_steal_ends_on_phase_out_and_does_not_revive_on_phase_in() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let sower = scenario
        .add_creature_to_hand_from_oracle(P0, "Sower of Temptation", 2, 2, SOWER_ORACLE)
        .id();
    let bear = scenario.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let mut runner = scenario.build();

    runner.cast(sower).target_object(bear).resolve();
    assert_eq!(
        runner.state().objects.get(&bear).unwrap().controller,
        P0,
        "reach-guard: the resolved ETB steal must be in force before the phase-out"
    );

    let mut events = Vec::new();
    phase_out_object(
        runner.state_mut(),
        sower,
        PhaseOutCause::Directly,
        &mut events,
    );
    assert!(
        !runner.state().objects.get(&sower).unwrap().is_phased_in(),
        "reach-guard: the production phase-out must actually phase Sower out"
    );
    evaluate_layers(runner.state_mut());
    assert_eq!(
        runner.state().objects.get(&bear).unwrap().controller,
        P1,
        "CR 702.26f: the presence-bound steal ends when Sower phases out"
    );
    assert!(
        runner
            .state()
            .transient_continuous_effects
            .iter()
            .all(|e| e.source_id != sower),
        "the steal must be ENDED (removed at the pruning seam), not merely suppressed"
    );

    phase_in_object(runner.state_mut(), sower, &mut events);
    assert!(
        runner.state().objects.get(&sower).unwrap().is_phased_in(),
        "reach-guard: Sower phased back in"
    );
    evaluate_layers(runner.state_mut());
    assert_eq!(
        runner.state().objects.get(&bear).unwrap().controller,
        P1,
        "CR 702.26f: the duration ENDED at the phase-out — phasing back in must not revive the steal"
    );
    assert_eq!(
        runner.state().objects.get(&sower).unwrap().zone,
        Zone::Battlefield,
        "CR 702.26d: phasing is not a zone change, so Sower never left the battlefield"
    );
}

#[test]
fn banisher_priests_event_deadline_exile_survives_its_phase_out() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let priest = scenario
        .add_creature_to_hand_from_oracle(P0, "Banisher Priest", 2, 2, BANISHER_PRIEST_ORACLE)
        .id();
    let bear = scenario.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let bolt = scenario.add_bolt_to_hand(P0);
    let mut runner = scenario.build();

    runner.cast(priest).target_object(bear).resolve();
    assert_eq!(
        runner.state().objects.get(&bear).unwrap().zone,
        Zone::Exile,
        "reach-guard: the ETB exile must have resolved"
    );

    let mut events = Vec::new();
    phase_out_object(
        runner.state_mut(),
        priest,
        PhaseOutCause::Directly,
        &mut events,
    );
    assert!(
        !runner.state().objects.get(&priest).unwrap().is_phased_in(),
        "reach-guard: the production phase-out must actually phase the Priest out"
    );
    evaluate_layers(runner.state_mut());
    assert_eq!(
        runner.state().objects.get(&bear).unwrap().zone,
        Zone::Exile,
        "CR 702.26d: a phase-out is not the Priest leaving the battlefield, so \
         the exiled creature must NOT return"
    );

    phase_in_object(runner.state_mut(), priest, &mut events);
    evaluate_layers(runner.state_mut());
    assert_eq!(
        runner.state().objects.get(&bear).unwrap().zone,
        Zone::Exile,
        "still exiled after the phase-in — the deadline never fired"
    );

    // Positive reach-guard for the pair machinery itself: when the Priest
    // ACTUALLY leaves the battlefield, the deadline fires and the creature
    // returns — proving the two phase-out assertions above were not vacuously
    // green on a dead exile/return link.
    runner.cast(bolt).target_object(priest).resolve();
    runner.advance_until_stack_empty();
    assert_eq!(
        runner.state().objects.get(&bear).unwrap().zone,
        Zone::Battlefield,
        "the Priest died, so \"until this creature leaves the battlefield\" ended \
         and the exiled creature returns"
    );
    assert_eq!(
        runner.state().objects.get(&bear).unwrap().controller,
        P1,
        "the returned creature is back under its owner's control"
    );
}
