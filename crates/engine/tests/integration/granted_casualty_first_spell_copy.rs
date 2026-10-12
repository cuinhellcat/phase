//! Anhelo, the Painter: "The first instant or sorcery spell you cast each turn
//! has casualty 2."
//!
//! The grant reads "spells cast this turn is zero", which stops holding once
//! the spell is cast. The copy trigger re-read the grant when the spell was
//! cast, so the sacrifice was paid and no copy was made. CR 702.153b: each
//! casualty instance "triggers based on the payments made for it".

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Anhelo, the Painter, verbatim.
const ANHELO: &str = "Deathtouch\nThe first instant or sorcery spell you cast each turn has \
casualty 2. (As you cast that spell, you may sacrifice a creature with power 2 or greater. When \
you do, copy the spell and you may choose new targets for the copy.)";

/// Cast a `{0}` "Draw a card." instant from hand with Anhelo and a 3/3 out,
/// answering casualty with `sacrifice`. Returns the runner and the 3/3.
fn cast_first_instant(sacrifice: bool) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_library_top(P0, &["Forest", "Forest", "Forest"]);
    scenario.add_creature_from_oracle(P0, "Anhelo, the Painter", 1, 1, ANHELO);
    let fodder = scenario.add_creature(P0, "Fodder", 3, 3).id();
    let mut instant = scenario.add_spell_to_hand_from_oracle(P0, "Zero Draw", true, "Draw a card.");
    instant.with_mana_cost(ManaCost::zero());
    let instant = instant.id();
    let mut runner = scenario.build();
    let card_id = runner.state().objects[&instant].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: instant,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("cast the instant");
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::OptionalCostChoice { .. }
        ),
        "reach guard: casualty is offered, found {:?}",
        runner.state().waiting_for
    );
    runner
        .act(GameAction::DecideOptionalCost { pay: sacrifice })
        .expect("decide casualty");
    if sacrifice {
        runner
            .act(GameAction::SelectCards {
                cards: vec![fodder],
            })
            .expect("sacrifice Fodder");
    }
    if matches!(runner.state().waiting_for, WaitingFor::ManaPayment { .. }) {
        runner.act(GameAction::PassPriority).expect("pay {0}");
    }
    assert_eq!(runner.state().objects[&instant].zone, Zone::Stack);
    (runner, fodder)
}

/// Pass priority until the stack is empty; returns the cards P0 drew.
fn resolve_and_count_draws(runner: &mut GameRunner) -> usize {
    let hand_before = runner.state().players[0].hand.len();
    for _ in 0..20 {
        if runner.state().stack.is_empty() {
            break;
        }
        runner.act(GameAction::PassPriority).expect("resolve");
    }
    assert!(runner.state().stack.is_empty());
    runner.state().players[0].hand.len() - hand_before
}

#[test]
fn paying_granted_casualty_on_the_first_instant_copies_it() {
    let (mut runner, fodder) = cast_first_instant(true);
    assert_eq!(runner.state().objects[&fodder].zone, Zone::Graveyard);
    assert_eq!(
        resolve_and_count_draws(&mut runner),
        2,
        "the instant and its casualty copy each draw a card"
    );
}

#[test]
fn declining_granted_casualty_makes_no_copy() {
    let (mut runner, fodder) = cast_first_instant(false);
    assert_eq!(runner.state().objects[&fodder].zone, Zone::Battlefield);
    assert_eq!(resolve_and_count_draws(&mut runner), 1);
}
