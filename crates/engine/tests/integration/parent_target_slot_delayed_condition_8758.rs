//! A delayed trigger whose CONDITION names an earlier declared target by slot
//! (`TargetFilter::ParentTargetSlot { index }`) watches that object and no
//! other. CR 603.7c: "A delayed triggered ability that refers to a particular
//! object still affects it even if the object changes characteristics.
//! However, if that object is no longer in the zone it's expected to be in at
//! the time the delayed triggered ability resolves, the ability won't affect
//! it."
//!
//! Stolen Uniform is the one printed carrier: "Choose target creature you
//! control and target Equipment. … When you lose control of that Equipment
//! this turn, if it's attached to a creature you control, unattach it." The
//! Equipment is slot 1 of the whole chain, but the clause that installs the
//! trigger is three instructions down, and each instruction only inherits its
//! immediate parent's targets. Bound against that one-element list, slot 1
//! was out of range and degraded to `TargetFilter::Any` (issue #8758), so the
//! trigger fired on the FIRST permanent you lost control of this turn,
//! whichever it was.
//!
//! The slot is now resolved through the shared chain-root slot authority,
//! `targeting::resolve_live_parent_slot_from_root`, which also carries the
//! CR 608.2b legality stamp: a slot whose target was illegal as the spell
//! resolved names nothing, and a condition that names nothing installs no
//! trigger, exactly as a bare `ParentTarget` over an empty parent set already
//! does.

use engine::game::game_object::AttachTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{DelayedTriggerCondition, EffectKind, TargetFilter};
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::keywords::Keyword;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;

use crate::rules::{cast_spell_action, drive_with_response, PriorityResponse};

const STOLEN_UNIFORM: &str = "Choose target creature you control and target Equipment. Gain \
control of that Equipment until end of turn. Attach it to the chosen creature. When you lose \
control of that Equipment this turn, if it's attached to a creature you control, unattach it.";

const STEAL: &str = "Gain control of target creature until end of turn.";

const ARTIFACT_HEXPROOF: &str = "Target artifact you control gains hexproof until end of turn.";

/// Reach guard: `source` resolved an instruction of `kind`, whatever it then
/// affected.
fn resolved(events: &[GameEvent], kind: EffectKind, source: ObjectId) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            GameEvent::EffectResolved { kind: resolved_kind, source_id, .. }
                if *resolved_kind == kind && *source_id == source
        )
    })
}

fn controller(runner: &GameRunner, id: ObjectId) -> PlayerId {
    runner.state().objects[&id].controller
}

fn attached_to(runner: &GameRunner, id: ObjectId) -> Option<AttachTarget> {
    runner.state().objects[&id].attached_to
}

/// The `valid_card` filter of the one installed `WhenNextEvent` delayed
/// trigger, or `None` when no delayed trigger is installed at all.
fn installed_valid_card(runner: &GameRunner) -> Option<TargetFilter> {
    let triggers = &runner.state().delayed_triggers;
    assert!(
        triggers.len() <= 1,
        "Stolen Uniform installs at most one delayed trigger, found {}",
        triggers.len()
    );
    let trigger = triggers.first()?;
    let DelayedTriggerCondition::WhenNextEvent { trigger, .. } = &trigger.condition else {
        panic!(
            "Stolen Uniform's delayed trigger is a WhenNextEvent, found {:?}",
            trigger.condition
        );
    };
    Some(
        trigger
            .valid_card
            .clone()
            .expect("the lose-control condition carries a valid_card filter"),
    )
}

/// Two creatures you control, an Equipment the opponent controls, Stolen
/// Uniform in your hand, and one instant (`response_oracle`) in the opponent's
/// hand, in your precombat main phase.
struct Board {
    runner: GameRunner,
    spell: ObjectId,
    mine: ObjectId,
    other: ObjectId,
    blade: ObjectId,
    response: ObjectId,
}

fn board(response_oracle: &str) -> Board {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mine = scenario.add_creature(P0, "Mine", 2, 2).id();
    let other = scenario.add_creature(P0, "Other", 1, 1).id();
    let blade = scenario
        .add_creature(P1, "Blade", 0, 1)
        .as_artifact()
        .with_subtypes(vec!["Equipment"])
        .id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Stolen Uniform", true, STOLEN_UNIFORM)
        .id();
    let response = scenario
        .add_spell_to_hand_from_oracle(P1, "Response", true, response_oracle)
        .id();
    Board {
        runner: scenario.build(),
        spell,
        mine,
        other,
        blade,
        response,
    }
}

/// Cast Stolen Uniform on (`mine`, `blade`) with no response and let it
/// resolve; returns the events. Reach-guards the attach so the later
/// assertions cannot pass on a spell that did nothing.
fn resolve_uniform(board: &mut Board) -> Vec<GameEvent> {
    let Board {
        runner,
        spell,
        mine,
        blade,
        ..
    } = board;
    let cast = cast_spell_action(runner, *spell);
    let events = drive_with_response(runner, cast, &[*mine, *blade], None);
    assert_eq!(
        controller(runner, *blade),
        P0,
        "reach guard: you gain control of the Equipment"
    );
    assert_eq!(
        attached_to(runner, *blade),
        Some(AttachTarget::Object(*mine)),
        "reach guard: the Equipment is attached to the chosen creature"
    );
    events
}

/// Hand priority to the opponent in your main phase with an empty stack, then
/// have them cast `instant` at `target` and let it resolve.
fn opponent_casts(runner: &mut GameRunner, instant: ObjectId, target: ObjectId) -> Vec<GameEvent> {
    runner
        .act(GameAction::PassPriority)
        .expect("pass priority to the opponent");
    assert_eq!(
        runner.state().waiting_for,
        WaitingFor::Priority { player: P1 },
        "reach guard: the opponent holds priority in your main phase"
    );
    let cast = cast_spell_action(runner, instant);
    drive_with_response(runner, cast, &[target], None)
}

/// Drive the current turn to its end and into the next turn's upkeep, passing
/// every priority window and declaring no attackers or blockers. Cleanup
/// (CR 514.2) is where the until-end-of-turn control ends.
fn advance_past_cleanup(runner: &mut GameRunner) {
    let turn = runner.state().turn_number;
    for _ in 0..64 {
        if runner.state().turn_number > turn && runner.state().phase == Phase::Upkeep {
            return;
        }
        let action = match &runner.state().waiting_for {
            WaitingFor::Priority { .. } => GameAction::PassPriority,
            WaitingFor::DeclareAttackers { .. } => GameAction::DeclareAttackers {
                attacks: vec![],
                bands: vec![],
            },
            WaitingFor::DeclareBlockers { .. } => GameAction::DeclareBlockers {
                assignments: vec![],
            },
            other => panic!("unexpected window while ending the turn: {other:?}"),
        };
        runner.act(action).expect("end the turn");
    }
    panic!("the next upkeep was not reached within the window budget");
}

/// CR 603.7c: the installed condition names the Equipment itself — slot 1 of
/// the chain — not `Any`.
#[test]
fn stolen_uniforms_delayed_trigger_names_the_equipment_it_took() {
    let mut board = board(STEAL);
    resolve_uniform(&mut board);

    assert_eq!(
        installed_valid_card(&board.runner),
        Some(TargetFilter::SpecificObject { id: board.blade }),
        "the lose-control condition is bound to the Equipment at declared slot 1"
    );
}

/// The game-visible half of the same fact. Losing control of a DIFFERENT
/// permanent this turn is not "losing control of that Equipment": the
/// Equipment stays attached and the trigger stays armed. At cleanup the
/// until-end-of-turn control ends (CR 514.2), you lose control of the
/// Equipment, and only then does it come off.
#[test]
fn losing_control_of_another_permanent_does_not_unattach_the_uniform() {
    let mut board = board(STEAL);
    resolve_uniform(&mut board);
    let Board {
        mut runner,
        mine,
        other,
        blade,
        response,
        ..
    } = board;

    opponent_casts(&mut runner, response, other);
    assert_eq!(
        controller(&runner, other),
        P1,
        "reach guard: the opponent stole the other creature"
    );
    assert_eq!(
        attached_to(&runner, blade),
        Some(AttachTarget::Object(mine)),
        "losing control of another permanent must not unattach the Equipment"
    );
    assert_eq!(
        runner.state().delayed_triggers.len(),
        1,
        "the one-shot trigger must still be armed for the Equipment's own control loss"
    );

    advance_past_cleanup(&mut runner);
    assert_eq!(
        controller(&runner, blade),
        P1,
        "reach guard: the until-end-of-turn control ended at cleanup"
    );
    assert_eq!(
        attached_to(&runner, blade),
        None,
        "losing control of the Equipment itself unattaches it"
    );
}

/// CR 608.2b: "If part of the effect requires information about an illegal
/// target, it fails to determine any such information. Any part of the effect
/// that requires that information won't happen." The Equipment gains
/// hexproof in response, so slot 1 is illegal as the spell resolves: you gain
/// control of nothing, attach nothing, and a trigger that would watch "that
/// Equipment" has nothing to watch — it is not installed, rather than
/// installed as a watch on every permanent.
#[test]
fn an_equipment_that_became_illegal_installs_no_delayed_trigger() {
    let Board {
        mut runner,
        spell,
        mine,
        blade,
        response,
        ..
    } = board(ARTIFACT_HEXPROOF);
    let cast = cast_spell_action(&runner, spell);
    let events = drive_with_response(
        &mut runner,
        cast,
        &[mine, blade],
        Some(PriorityResponse {
            player: P1,
            instant: response,
            target: blade,
        }),
    );

    assert!(
        runner.state().objects[&blade].has_keyword(&Keyword::Hexproof),
        "reach guard: the response gave the Equipment hexproof"
    );
    assert!(
        resolved(&events, EffectKind::GainControl, spell),
        "reach guard: the creature you control is still legal, so the spell resolves"
    );
    assert_eq!(
        controller(&runner, blade),
        P1,
        "reach guard: an illegal Equipment stays under its controller"
    );
    assert!(
        resolved(&events, EffectKind::CreateDelayedTrigger, spell),
        "reach guard: the installing clause ran (the refusal reports the same event as an \
         install), so the missing trigger is a refusal, not a chain that stopped early"
    );
    assert_eq!(
        installed_valid_card(&runner),
        None,
        "a condition naming an illegal slot installs no delayed trigger"
    );
}
