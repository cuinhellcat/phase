//! Regression for GitHub issue #7539 — the sandbox `Turn Face Up` action must
//! RESTORE the stored face, not just clear the flag.
//!
//! CR 708.2a: a face-down permanent is a 2/2 creature with no name, no mana
//! cost, no creature types and no abilities. Its real characteristics live in
//! `back_face` until it is turned face up. CR 702.37e: the morph effect ends
//! and the permanent "regains its normal characteristics". Clearing `face_down`
//! alone
//! leaves the vanilla 2/2 installed, so the tool appears to do nothing.
//!
//! Same class as #3284 / #3290, where the debug `transformed` write was routed
//! through `transform::transform_permanent` by #3684. The `face_down` write in
//! the same match arm was never carried over.

use engine::game::scenario::{GameScenario, P0};
use engine::types::actions::{DebugAction, GameAction};
use engine::types::events::GameEvent;
use engine::types::mana::{ManaCost, ManaCostShard};
use engine::types::zones::Zone;

/// A creature card in hand with a real mana cost, so CR 701.40b can derive the
/// turn-face-up cost from the stored face.
fn board() -> (
    engine::game::scenario::GameRunner,
    engine::types::identifiers::ObjectId,
) {
    let mut scenario = GameScenario::new();
    let id = scenario
        .add_creature_to_hand(P0, "Hidden Bear", 3, 3)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Green],
            generic: 1,
        })
        .id();
    let mut runner = scenario.build();
    runner.state_mut().debug_mode = true;

    let mut events = Vec::new();
    engine::game::morph::play_face_down(runner.state_mut(), P0, id, &mut events)
        .expect("the card is played face down");

    let obj = &runner.state().objects[&id];
    assert!(obj.face_down, "setup: the permanent is face down");
    assert_eq!(obj.zone, Zone::Battlefield);
    assert_eq!(obj.name, "", "CR 708.2a: a face-down permanent has no name");
    assert_eq!(obj.base_power, Some(2), "CR 708.2a: it is a 2/2");

    (runner, id)
}

/// The defect: the tool must produce the real card, and it must produce the
/// event the turn-face-up triggers observe.
#[test]
fn the_sandbox_turn_face_up_restores_the_stored_face() {
    let (mut runner, id) = board();

    let result = runner
        .act(GameAction::Debug(DebugAction::SetFaceState {
            object_id: id,
            face_down: Some(false),
            transformed: None,
            flipped: None,
        }))
        .expect("the debug turn-face-up runs");

    let obj = &runner.state().objects[&id];
    assert!(!obj.face_down);
    assert_eq!(obj.name, "Hidden Bear", "the stored face is restored");
    assert_eq!(
        (obj.base_power, obj.base_toughness),
        (Some(3), Some(3)),
        "with its printed power and toughness, not the CR 708.2a 2/2"
    );

    // The discriminating assertion. A flag-only write also leaves `face_down`
    // false, so the flag alone cannot tell the two implementations apart — the
    // restored characteristics and this event can. `TurnedFaceUp` is what the
    // "when this is turned face up" triggers and the
    // "as ~ is turned face up" replacement key on; without it the tool changes a
    // flag and the game never learns anything happened.
    assert!(
        result.events.iter().any(
            |event| matches!(event, GameEvent::TurnedFaceUp { object_id, .. } if *object_id == id)
        ),
        "the turn-face-up event must reach the triggers, got {:?}",
        result.events
    );
}

/// Counter-direction: an object with no stored face keeps the plain flag write,
/// so the arm stays a debug tool for states the rules cannot reach.
#[test]
fn a_permanent_without_a_stored_face_keeps_the_plain_flag_write() {
    let mut scenario = GameScenario::new();
    let id = scenario.add_creature(P0, "Ordinary Bear", 2, 2).id();
    let mut runner = scenario.build();
    runner.state_mut().debug_mode = true;
    runner.state_mut().objects.get_mut(&id).unwrap().face_down = true;

    runner
        .act(GameAction::Debug(DebugAction::SetFaceState {
            object_id: id,
            face_down: Some(false),
            transformed: None,
            flipped: None,
        }))
        .expect("the debug write runs");

    let obj = &runner.state().objects[&id];
    assert!(!obj.face_down);
    assert_eq!(obj.name, "Ordinary Bear");
}
