//! Issue #7822: manifesting a planeswalker card seeded LOYALTY COUNTERS onto
//! the face-down permanent and left `obj.loyalty` set — the client renders the
//! loyalty badge on a face-down 2/2, leaking that the card is a planeswalker.
//! CR 708.2a: a face-down permanent is a 2/2 creature — it has no loyalty (or
//! defense) characteristic; CR 306.5b seeds loyalty only for a planeswalker
//! entering as one.
//!
//! REVERT DISCRIMINATOR: without the loyalty/defense blanking in
//! `apply_face_down_creature_characteristics`, `intrinsic_etb_counters` reads
//! the card's printed loyalty during the face-down entry and the
//! no-loyalty-counter assertion fails.

use engine::game::scenario::{GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::card::PrintedLoyalty;
use engine::types::card_type::CoreType;
use engine::types::counter::CounterType;
use engine::types::game_state::WaitingFor;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const MANIFEST_DREAD: &str = "Manifest dread.";

#[test]
fn a_manifested_planeswalker_card_has_no_loyalty() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let walker = scenario.add_card_to_library_top(P0, "Buried Walker");
    scenario.add_card_to_library_top(P0, "Second Top");
    let spell = scenario
        .add_spell_to_hand(P0, "Dread Test", false)
        .from_oracle_text(MANIFEST_DREAD)
        .with_mana_cost(ManaCost::generic(0))
        .id();
    scenario.with_mana_pool(P0, vec![]);

    let mut runner = scenario.build();
    {
        let obj = runner.state_mut().objects.get_mut(&walker).unwrap();
        obj.card_types.core_types.push(CoreType::Planeswalker);
        obj.loyalty = Some(4);
        obj.printed_loyalty = Some(PrintedLoyalty::Fixed(4));
    }
    runner.cast(spell).resolve();
    let WaitingFor::ManifestDreadChoice { .. } = runner.state().waiting_for.clone() else {
        panic!(
            "manifest dread must pause for a card choice, got {:?}",
            runner.state().waiting_for
        );
    };
    runner
        .act(GameAction::SelectCards {
            cards: vec![walker],
        })
        .expect("manifest choice must be accepted");
    runner.advance_until_stack_empty();

    let obj = runner
        .state()
        .objects
        .get(&walker)
        .expect("manifested object exists");
    assert_eq!(obj.zone, Zone::Battlefield);
    assert!(obj.face_down, "manifested object is face down");
    assert_eq!(
        obj.counters.get(&CounterType::Loyalty),
        None,
        "no loyalty counters may be seeded on a face-down entry (CR 708.2a)"
    );
    assert_eq!(
        obj.loyalty, None,
        "the face-down permanent has no loyalty characteristic to display"
    );
    assert_eq!(obj.defense, None);
    // The real card survives underneath for turn-up restoration.
    let back = obj.back_face.as_ref().expect("back face snapshot exists");
    assert_eq!(
        back.loyalty,
        Some(4),
        "the snapshot keeps the printed value"
    );
}
