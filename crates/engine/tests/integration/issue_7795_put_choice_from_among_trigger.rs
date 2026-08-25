//! Issue #7795 (Aragorn, Company Leader) — the REAL Ring-tempts pipeline:
//! `Effect::RingTemptsYou` → `WaitingFor::ChooseRingBearer` →
//! `state.ring_bearer` write → batched `TriggerMode::RingTemptsYou` observer
//! drain → intervening-if `ChoseOtherRingBearer` → four-kind
//! `ChooseOneOfBranch` → the picked counter folds onto Aragorn.
//!
//! Aragorn's first printed line verbatim (the second line's counter
//! reproduction is out of scope here). REVERT DISCRIMINATORS:
//! - without the AST-route `try_parse_put_counter_choice` call the trigger
//!   body is `Unimplemented` — no `ChooseOneOfBranch` ever appears;
//! - without `TriggerCondition::ChoseOtherRingBearer` the intervening-if is
//!   dropped — `choosing_aragorn_himself_offers_no_counter_choice` sees the
//!   counter prompt it must not see.

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::keywords::{Keyword, KeywordKind};
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;

const ARAGORN: &str = "Whenever the Ring tempts you, if you chose a creature other than Aragorn as your Ring-bearer, put your choice of a counter from among first strike, vigilance, deathtouch, and lifelink on Aragorn.";
const TEMPT: &str = "The Ring tempts you.";

fn keyword_counter(runner: &GameRunner, object: ObjectId, kind: KeywordKind) -> u32 {
    runner
        .state()
        .objects
        .get(&object)
        .and_then(|card| card.counters.get(&CounterType::Keyword(kind)).copied())
        .unwrap_or(0)
}

/// Drive the temptation to a settled board. Every prompt is answered or the
/// test dies: an unexpected prompt and a rejected action both panic, and the
/// loop must REACH the empty-stack terminal — falling off the iteration bound
/// is a failure, so "no counter choice offered" can never be misreported.
fn drive_temptation(runner: &mut GameRunner, bearer: ObjectId, counter_index: usize) -> bool {
    let mut counter_choice_seen = false;
    for _ in 0..64 {
        match runner.state().waiting_for.clone() {
            WaitingFor::ChooseRingBearer { candidates, .. } => {
                assert!(
                    candidates.contains(&bearer),
                    "intended bearer must be a legal candidate, got {candidates:?}"
                );
                runner
                    .act(GameAction::ChooseRingBearer { target: bearer })
                    .expect("the Ring-bearer choice must be accepted");
            }
            WaitingFor::ChooseOneOfBranch { branches, .. } => {
                assert_eq!(branches.len(), 4, "all four counter kinds must be offered");
                runner
                    .act(GameAction::ChooseBranch {
                        index: counter_index,
                    })
                    .expect("choosing a counter kind must succeed");
                counter_choice_seen = true;
            }
            WaitingFor::Priority { .. } => {
                if runner.state().stack.is_empty() {
                    return counter_choice_seen;
                }
                runner
                    .act(GameAction::PassPriority)
                    .expect("PassPriority must be accepted mid-drive");
            }
            other => panic!("unexpected prompt during the temptation: {other:?}"),
        }
    }
    panic!("temptation never settled to an empty stack within 64 steps");
}

fn tempted_board() -> (GameRunner, ObjectId, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let aragorn = scenario
        .add_creature_from_oracle(P0, "Aragorn, Company Leader", 2, 2, ARAGORN)
        .id();
    let companion = scenario.add_creature(P0, "Companion Hobbit", 1, 1).id();
    let tempt = scenario
        .add_spell_to_hand(P0, "Temptation Test", false)
        .from_oracle_text(TEMPT)
        .with_mana_cost(ManaCost::generic(0))
        .id();
    scenario.with_mana_pool(P0, vec![]);
    let mut runner = scenario.build();
    runner.cast(tempt).resolve();
    (runner, aragorn, companion, tempt)
}

#[test]
fn choosing_another_bearer_offers_the_choice_and_folds_the_pick() {
    let (mut runner, aragorn, companion, _) = tempted_board();

    let offered = drive_temptation(&mut runner, companion, 3);

    assert!(offered, "the counter-kind choice must be offered");
    assert_eq!(
        runner.state().ring_bearer.get(&P0).copied().flatten(),
        Some(companion),
        "the chosen companion must be the Ring-bearer"
    );
    assert_eq!(keyword_counter(&runner, aragorn, KeywordKind::Lifelink), 1);
    assert_eq!(
        keyword_counter(&runner, aragorn, KeywordKind::FirstStrike),
        0,
        "unchosen kinds must not be folded"
    );
    let obj = runner
        .state()
        .objects
        .get(&aragorn)
        .expect("Aragorn exists");
    assert!(obj.has_keyword(&Keyword::Lifelink));
    assert!(!obj.has_keyword(&Keyword::FirstStrike));
}

#[test]
fn a_different_pick_folds_only_that_kind() {
    let (mut runner, aragorn, companion, _) = tempted_board();

    assert!(drive_temptation(&mut runner, companion, 0));
    assert_eq!(
        keyword_counter(&runner, aragorn, KeywordKind::FirstStrike),
        1
    );
    assert_eq!(keyword_counter(&runner, aragorn, KeywordKind::Lifelink), 0);
}

/// CR 603.4 intervening-if negative + reach-guard pair: choosing ARAGORN
/// himself must not offer the counter choice (the positive tests prove the
/// same fixture DOES reach it for another bearer, so this cannot pass
/// vacuously).
#[test]
fn choosing_aragorn_himself_offers_no_counter_choice() {
    let (mut runner, aragorn, _companion, _) = tempted_board();

    let offered = drive_temptation(&mut runner, aragorn, 0);

    assert!(
        !offered,
        "no counter choice may be offered when Aragorn is his own bearer"
    );
    assert_eq!(
        runner.state().ring_bearer.get(&P0).copied().flatten(),
        Some(aragorn)
    );
    for kind in [
        KeywordKind::FirstStrike,
        KeywordKind::Vigilance,
        KeywordKind::Deathtouch,
        KeywordKind::Lifelink,
    ] {
        assert_eq!(keyword_counter(&runner, aragorn, kind), 0);
    }
}
