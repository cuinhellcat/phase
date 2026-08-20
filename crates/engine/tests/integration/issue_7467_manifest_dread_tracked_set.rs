//! #7467 (ManifestDread row): "Manifest dread X times, then put X +1/+1
//! counters on each of those creatures" — Valgavoth's Onslaught.
//!
//! Manifest dread with two or more library cards parks
//! `WaitingFor::ManifestDreadChoice`, so the manifested creature enters from
//! the CONTINUATION handler and its `ZoneChanged` lands on the continuation's
//! event vector. The resolver-side tracked-set publish (CR 603.7 class,
//! `effects/mod.rs`) harvests only the resolver's own events — it published an
//! EMPTY set, and the chained `PutCounterAll { TrackedSet }` ("each of those
//! creatures") bound nothing. Cast with X > 0, the manifested creatures got no
//! counters at all.
//!
//! The catalog parse chains the counter sub-ability inside each `repeat_for`
//! iteration, so each creature receives its X counters before the next
//! iteration manifests — the FINAL board (every creature manifested this way
//! carries X counters) is what the printed text requires, and what these rows
//! measure.

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;

const ONSLAUGHT: &str =
    "Manifest dread X times, then put X +1/+1 counters on each of those creatures.";

/// A fabricated Onslaught in hand ({X} cost) plus `library` library cards so
/// manifest dread has something to look at.
fn board(library: usize) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    for i in 0..library {
        scenario.add_card_to_library_top(P0, &format!("Library {i}"));
    }
    let spell = {
        let mut b =
            scenario.add_spell_to_hand_from_oracle(P0, "Fabricated Onslaught", false, ONSLAUGHT);
        b.with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::X],
            generic: 0,
        });
        b.id()
    };
    let pool = (0..4)
        .map(|_| ManaUnit::new(ManaType::Colorless, ObjectId(0), false, vec![]))
        .collect();
    scenario.with_mana_pool(P0, pool);
    (scenario.build(), spell)
}

fn p1p1_counters(runner: &GameRunner, id: ObjectId) -> u32 {
    runner.state().objects[&id]
        .counters
        .get(&CounterType::Plus1Plus1)
        .copied()
        .unwrap_or(0)
}

/// The continuation arm (the #7467 gap): X=2 with four library cards parks the
/// two-card choice twice; each chosen creature must end with X = 2 counters.
#[test]
fn each_creature_manifested_through_the_choice_gets_x_counters() {
    let (mut runner, spell) = board(4);
    runner.cast(spell).x(2).resolve();
    runner.advance_until_stack_empty();

    let mut manifested = Vec::new();
    for round in 0..2 {
        let offered = match &runner.state().waiting_for {
            WaitingFor::ManifestDreadChoice { cards, .. } => cards.clone(),
            other => panic!("round {round}: expected the manifest dread choice, got {other:?}"),
        };
        assert_eq!(offered.len(), 2, "round {round}: two cards to choose from");
        let pick = offered[0];
        runner
            .act(GameAction::SelectCards { cards: vec![pick] })
            .expect("choose the card to manifest");
        manifested.push(pick);
        runner.advance_until_stack_empty();
    }

    for &id in &manifested {
        let obj = &runner.state().objects[&id];
        assert!(
            obj.face_down,
            "the chosen card must sit face down on the battlefield"
        );
        assert_eq!(
            p1p1_counters(&runner, id),
            2,
            "X=2: every creature manifested this way must carry X +1/+1 counters \
             (#7467: the choice continuation published no tracked set)"
        );
    }
}

/// The synchronous arm (control): a one-card library skips the choice —
/// `manifest_dread.rs` manifests directly and the resolver-side harvest
/// already publishes the set. Exactly X = 1 counter, before AND after the
/// continuation publish exists: the fix must not double-apply here.
#[test]
fn a_one_card_library_manifests_synchronously_with_exactly_x_counters() {
    let (mut runner, spell) = board(1);
    runner.cast(spell).x(1).resolve();
    runner.advance_until_stack_empty();

    let manifested = runner
        .state()
        .battlefield
        .iter()
        .copied()
        .find(|id| runner.state().objects[id].face_down)
        .expect("the single library card must be manifested without a choice");
    assert_eq!(
        p1p1_counters(&runner, manifested),
        1,
        "X=1 on the synchronous arm: exactly one counter — no more (double publish), no less"
    );
}
