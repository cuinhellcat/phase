//! Issue #7795 (Aragorn, Company Leader): a triggered ability with an
//! intervening "if" whose body is "put your choice of a counter from among
//! first strike, vigilance, deathtouch, and lifelink on ~" lowers through the
//! AST imperative route, which used to swallow the clause as `Unimplemented`.
//! Runtime proof over the real pipeline: trigger fires → the four-kind
//! `ChooseOneOfBranch` is offered → exactly the chosen counter folds.
//!
//! The trigger here is a life-gain stand-in for the Ring-tempts head (same
//! effect-body route — the head does not change the body's parse path; the
//! intervening "if" does). REVERT DISCRIMINATOR: without the AST-route
//! `try_parse_put_counter_choice` call the body parses as `Unimplemented`,
//! the resolved trigger never pauses, and the `ChooseOneOfBranch` assertion
//! fails.

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::keywords::{Keyword, KeywordKind};
use engine::types::phase::Phase;

const BEARER: &str = "Whenever you gain life, if you control an artifact, put your choice of a counter from among first strike, vigilance, deathtouch, and lifelink on ~.";
const GAIN: &str = "You gain 3 life.";

fn counter_count(runner: &GameRunner, object: ObjectId, kind: KeywordKind) -> u32 {
    runner
        .state()
        .objects
        .get(&object)
        .and_then(|card| card.counters.get(&CounterType::Keyword(kind)).copied())
        .unwrap_or(0)
}

fn cast_spell(runner: &mut GameRunner, spell: ObjectId) {
    let card_id = runner
        .state()
        .objects
        .get(&spell)
        .expect("spell object exists")
        .card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("cast must be accepted");
}

/// Drive to stack-empty; when the counter choice appears, pick `index` after
/// asserting all four kinds are offered. Returns whether a choice was offered.
fn drive_and_choose(runner: &mut GameRunner, index: usize) -> bool {
    let mut chosen = false;
    for _ in 0..64 {
        let wf = runner.state().waiting_for.clone();
        match wf {
            WaitingFor::ChooseOneOfBranch { branches, .. } => {
                assert_eq!(branches.len(), 4, "all four counter kinds offered");
                runner
                    .act(GameAction::ChooseBranch { index })
                    .expect("choosing a counter kind must succeed");
                chosen = true;
            }
            WaitingFor::Priority { .. } => {
                if runner.state().stack.is_empty() {
                    break;
                }
                if runner.act(GameAction::PassPriority).is_err() {
                    break;
                }
            }
            other => panic!("unexpected prompt: {other:?}"),
        }
    }
    chosen
}

fn scenario_with_bearer(with_artifact: bool) -> (GameRunner, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let bearer = scenario
        .add_creature_from_oracle(P0, "Choice Bearer", 2, 2, BEARER)
        .id();
    if with_artifact {
        scenario.add_artifact_from_oracle(P0, "Plain Trinket", "");
    }
    let gain = scenario
        .add_spell_to_hand_from_oracle(P0, "Restorative Draught", true, GAIN)
        .id();
    (scenario.build(), bearer, gain)
}

#[test]
fn the_resolved_trigger_offers_four_kinds_and_folds_the_pick() {
    let (mut runner, bearer, gain) = scenario_with_bearer(true);
    cast_spell(&mut runner, gain);
    let chosen = drive_and_choose(&mut runner, 3);

    assert!(chosen, "the counter-kind choice must be offered");
    assert_eq!(counter_count(&runner, bearer, KeywordKind::Lifelink), 1);
    assert_eq!(counter_count(&runner, bearer, KeywordKind::FirstStrike), 0);
    assert_eq!(counter_count(&runner, bearer, KeywordKind::Vigilance), 0);
    assert_eq!(counter_count(&runner, bearer, KeywordKind::Deathtouch), 0);
    let obj = runner.state().objects.get(&bearer).expect("bearer exists");
    assert!(obj.has_keyword(&Keyword::Lifelink));
    assert!(
        !obj.has_keyword(&Keyword::FirstStrike),
        "unchosen kinds must not be granted"
    );
}

#[test]
fn a_different_pick_folds_only_that_kind() {
    let (mut runner, bearer, gain) = scenario_with_bearer(true);
    cast_spell(&mut runner, gain);
    let chosen = drive_and_choose(&mut runner, 0);

    assert!(chosen, "the counter-kind choice must be offered");
    assert_eq!(counter_count(&runner, bearer, KeywordKind::FirstStrike), 1);
    assert_eq!(counter_count(&runner, bearer, KeywordKind::Lifelink), 0);
}

/// Negative + reach-guard pair: with the intervening "if" unsatisfied (no
/// artifact), the fired trigger must fizzle without offering the choice. The
/// positive tests above prove the same fixture DOES reach the choice, so this
/// cannot pass vacuously.
#[test]
fn an_unsatisfied_intervening_if_offers_no_choice() {
    let (mut runner, bearer, gain) = scenario_with_bearer(false);
    cast_spell(&mut runner, gain);
    let chosen = drive_and_choose(&mut runner, 0);

    assert!(!chosen, "no choice may be offered when the if fails");
    assert_eq!(counter_count(&runner, bearer, KeywordKind::FirstStrike), 0);
    assert_eq!(counter_count(&runner, bearer, KeywordKind::Lifelink), 0);
}
