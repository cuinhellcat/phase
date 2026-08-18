//! CR 116.2b + CR 702.37e: the turn-face-up special action must be OFFERED, not
//! merely accepted (#6732, #4381).
//!
//! The engine implemented the action and its Priority preflight counted it as
//! progress, but `ai_support::candidates::priority_actions_with_probe` — the
//! list the client renders — never emitted it. Nothing could send an action the
//! engine never advertised, so the whole morph / megamorph / disguise / manifest
//! / cloak class was unturnable in play. #7342 wired the client's dispatch and
//! closed both reports; its test supplies the action to itself, so it proves the
//! client's half and cannot observe the engine's.
//!
//! Reported from a real game state: the controller had priority, a face-down
//! Coral Trickster (morph `{U}`) and thirty untapped Islands, and `legalActions`
//! held six casts, four land plays and a pass.

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{
    AbilityCost, AbilityDefinition, AbilityKind, Effect, ManaContribution, ManaProduction,
    ReplacementDefinition, TargetFilter,
};
use engine::types::actions::GameAction;
use engine::types::game_state::{ManaAbilityResume, PendingCostMoveResume, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::keywords::Keyword;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::replacements::ReplacementEvent;
use engine::types::zones::{EtbTapState, Zone};

fn morph_cost() -> ManaCost {
    ManaCost::Cost {
        shards: vec![ManaCostShard::Blue],
        generic: 0,
    }
}

fn pool(kinds: &[ManaType]) -> Vec<ManaUnit> {
    kinds
        .iter()
        .copied()
        .map(|kind| ManaUnit::new(kind, ObjectId(0), false, vec![]))
        .collect()
}

/// A face-down permanent with a morph cost, put onto the battlefield through the
/// engine's own face-down play so `back_face` carries the real card.
fn face_down_morph_board(controller: PlayerId, mana: &[ManaType]) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let id = scenario
        .add_creature_to_hand(controller, "Coral Trickster", 2, 1)
        .with_keyword(Keyword::Morph(morph_cost()))
        .id();
    if !mana.is_empty() {
        scenario.with_mana_pool(controller, pool(mana));
    }
    let mut runner = scenario.build();

    let mut events = Vec::new();
    engine::game::morph::play_face_down(runner.state_mut(), controller, id, &mut events)
        .expect("the card is played face down");
    assert!(
        runner.state().objects[&id].face_down,
        "setup: the permanent is face down"
    );

    (runner, id)
}

fn offered_turn_face_ups(runner: &GameRunner) -> Vec<ObjectId> {
    engine::ai_support::legal_actions(runner.state())
        .into_iter()
        .filter_map(|action| match action {
            GameAction::TurnFaceUp { object_id, .. } => Some(object_id),
            _ => None,
        })
        .collect()
}

/// The defect, end to end: the action is offered, and taking the offer flips the
/// permanent to its real face.
#[test]
fn a_face_down_morph_permanent_is_offered_and_flips() {
    let (mut runner, id) = face_down_morph_board(P0, &[ManaType::Blue]);

    assert_eq!(
        offered_turn_face_ups(&runner),
        vec![id],
        "CR 116.2b: the controller has priority and can pay {{U}}, so the special \
         action must be on the list the client renders"
    );

    runner
        .act(GameAction::TurnFaceUp {
            object_id: id,
            x: 0,
        })
        .expect("the offered action must be accepted");

    let obj = &runner.state().objects[&id];
    assert!(!obj.face_down, "CR 702.37e: the permanent is now face up");
    assert_eq!(obj.name, "Coral Trickster", "and shows its real face");
}

/// The affordability half of the same authority: an unpayable cost is not
/// offered, so the list never advertises an action the reducer would reject.
///
/// This is also what keeps the row above from passing for the wrong reason — if
/// the offer were unconditional, this row would fail.
#[test]
fn an_unpayable_turn_face_up_is_not_offered() {
    let (runner, _) = face_down_morph_board(P0, &[]);

    assert!(
        offered_turn_face_ups(&runner).is_empty(),
        "with no mana the morph cost cannot be paid, so nothing is offered"
    );
}

/// CR 702.37e: "you may turn a face-down permanent YOU CONTROL face up". The
/// offer is per-holder, and `legal_actions` speaks for the priority holder.
#[test]
fn an_opponents_face_down_permanent_is_not_offered() {
    let (mut runner, _) = face_down_morph_board(P1, &[ManaType::Blue]);
    runner.state_mut().priority_player = P0;
    runner.state_mut().waiting_for = WaitingFor::Priority { player: P0 };

    assert!(
        offered_turn_face_ups(&runner).is_empty(),
        "a face-down permanent an opponent controls is not this player's to turn up"
    );
}

// ── The paused mana-source payment (#4538's blocker) ────────────────────────

fn redirect_exile_to_graveyard() -> ReplacementDefinition {
    ReplacementDefinition::new(ReplacementEvent::Moved)
        .destination_zone(Zone::Exile)
        .execute(AbilityDefinition::new(
            AbilityKind::Spell,
            Effect::ChangeZone {
                destination: Zone::Graveyard,
                origin: None,
                target: TargetFilter::SelfRef,
                owner_library: false,
                enter_transformed: false,
                enters_under: None,
                enter_tapped: EtbTapState::Unspecified,
                enters_attacking: false,
                up_to: false,
                enter_with_counters: vec![],
                conditional_enter_with_counters: vec![],
                face_down_profile: None,
                enters_modified_if: None,
            },
        ))
}

/// CR 605.3b + CR 616.1: the offer is only honest if the action can FINISH.
///
/// A mana source whose own cost exiles it, plus two exile→graveyard
/// replacements, forces a CR 616.1 ordering choice while the turn-face-up cost
/// is being auto-tapped. `casting.rs` deliberately reports such a source as
/// payable, so the offer above is right to include it — but the compatibility
/// wrapper `pay_special_action_mana_cost` converts the resulting `Paused` into
/// an error. That is why #4538 was asked to build a typed resume before the
/// action could be offered.
///
/// The permanent must still be face down while the choice is open, and must flip
/// exactly once the choice is answered.
#[test]
fn a_paused_mana_source_resumes_the_locked_turn_face_up() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let id = scenario
        .add_creature_to_hand(P0, "Coral Trickster", 2, 1)
        .with_keyword(Keyword::Morph(morph_cost()))
        .id();
    let source = scenario
        .add_creature(P0, "Self-Exiling Mana Source", 1, 1)
        .with_ability_definition(
            AbilityDefinition::new(
                AbilityKind::Activated,
                Effect::Mana {
                    produced: ManaProduction::Fixed {
                        colors: vec![ManaColor::Blue],
                        contribution: ManaContribution::Base,
                    },
                    restrictions: vec![],
                    grants: vec![],
                    expiry: None,
                    target: None,
                },
            )
            .cost(AbilityCost::Composite {
                costs: vec![
                    AbilityCost::Tap,
                    AbilityCost::Exile {
                        count: 1,
                        zone: None,
                        filter: Some(TargetFilter::SelfRef),
                    },
                ],
            }),
        )
        .id();
    for name in ["First Pause Replacement", "Second Pause Replacement"] {
        scenario
            .add_creature(P0, name, 0, 0)
            .as_enchantment()
            .with_replacement_definition(redirect_exile_to_graveyard());
    }
    let mut runner = scenario.build();

    let mut events = Vec::new();
    engine::game::morph::play_face_down(runner.state_mut(), P0, id, &mut events)
        .expect("the card is played face down");

    assert_eq!(
        offered_turn_face_ups(&runner),
        vec![id],
        "the auto-tap probe finds the self-exiling source, so the action is offered"
    );

    let paused = runner
        .act(GameAction::TurnFaceUp {
            object_id: id,
            x: 0,
        })
        .expect("the source's own cost pauses the payment rather than failing it");
    assert!(
        matches!(paused.waiting_for, WaitingFor::ReplacementChoice { .. }),
        "the mana source's exile replacement owns the window, got {:?}",
        paused.waiting_for
    );
    assert!(
        matches!(
            runner.state().pending_cost_move_resume.as_ref(),
            Some(PendingCostMoveResume::ManaAbilityPayment { pending, .. })
                if matches!(
                    &pending.resume,
                    ManaAbilityResume::TurnFaceUp { player, object_id, .. }
                        if *player == P0 && *object_id == id
                )
        ),
        "the typed continuation names the action to finish"
    );
    assert!(
        runner.state().objects[&id].face_down,
        "nothing is committed while the payment is open"
    );

    runner
        .act(GameAction::ChooseReplacement { index: 0 })
        .expect("the replacement choice is answered");

    let obj = &runner.state().objects[&id];
    assert!(
        !obj.face_down,
        "CR 605.3b: the locked payment completed and the flip committed"
    );
    assert_eq!(obj.name, "Coral Trickster");
    assert_eq!(
        runner.state().objects[&source].zone,
        Zone::Graveyard,
        "the mana source's own cost still resolved through its replacement"
    );
}
