//! CR 702.37b: megamorph's paid-turn-up counter rider, end to end.
//!
//! "Megamorph [cost]" means … "As this permanent is turned face up, put a
//! +1/+1 counter on it if its megamorph cost was paid to turn it face up."
//! Reported live: megamorph creatures came up counter-less. The rider is a
//! keyword-synthesized `TurnFaceUp` replacement gated on the payment fact the
//! PAID special action publishes — so it orders with any other
//! as-turned-face-up replacement (CR 616.1) and never fires for a free,
//! effect-driven turn-up.
//!
//! The printed costs are deliberately unpayable ({9}) so the face-down {3}
//! alternative (CR 708.4) is the only castable route.

use engine::game::scenario::{GameScenario, P0};
use engine::types::actions::{AlternativeCastDecision, GameAction};
use engine::types::counter::CounterType;
use engine::types::game_state::WaitingFor;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

struct FlipOutcome {
    prompts: Vec<String>,
    counters: Option<u32>,
    face_down: Option<bool>,
}

/// Cast the card face down (the only affordable route), settle, then take the
/// paid `TurnFaceUp` special action and settle again.
fn cast_face_down_and_flip(oracle_text: &str, mana: u32) -> FlipOutcome {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Hidden Test Creature", false, oracle_text)
        .as_creature()
        .id();
    scenario.with_mana_pool(
        P0,
        (0..mana)
            .map(|_| {
                engine::types::mana::ManaUnit::new(
                    engine::types::mana::ManaType::Green,
                    engine::types::identifiers::ObjectId(0),
                    false,
                    vec![],
                )
            })
            .collect(),
    );

    let mut runner = scenario.build();
    runner
        .cast(spell)
        .alternative_cast(AlternativeCastDecision::Alternative)
        .commit();

    let mut prompts = Vec::new();
    let mut settled = false;
    for _ in 0..40 {
        match runner.state().waiting_for.clone() {
            WaitingFor::Priority { .. } if runner.state().stack.is_empty() => {
                settled = true;
                break;
            }
            WaitingFor::Priority { .. } => {
                if runner.act(GameAction::PassPriority).is_err() {
                    break;
                }
            }
            other => {
                prompts.push(format!("PROMPT: {}", other.variant_name()));
                break;
            }
        }
    }
    assert!(settled, "the face-down cast must settle — {prompts:?}");

    let object_id = runner
        .state()
        .objects
        .values()
        .find(|o| o.zone == Zone::Battlefield && o.face_down)
        .map(|o| o.id)
        .expect("a face-down 2/2 must be on the battlefield");

    let flip = runner.act(GameAction::TurnFaceUp { object_id, x: 0 });
    assert!(flip.is_ok(), "the paid turn-up must be legal: {flip:?}");

    let mut settled = false;
    for _ in 0..40 {
        match runner.state().waiting_for.clone() {
            WaitingFor::Priority { .. } if runner.state().stack.is_empty() => {
                settled = true;
                break;
            }
            WaitingFor::Priority { .. } => {
                if runner.act(GameAction::PassPriority).is_err() {
                    break;
                }
            }
            other => {
                prompts.push(format!("PROMPT: {}", other.variant_name()));
                break;
            }
        }
    }
    assert!(settled, "the flip must settle — {prompts:?}");

    let obj = runner.state().objects.get(&object_id);
    FlipOutcome {
        prompts,
        counters: obj.map(|o| {
            o.counters
                .get(&CounterType::Plus1Plus1)
                .copied()
                .unwrap_or(0)
        }),
        face_down: obj.map(|o| o.face_down),
    }
}

/// CR 702.37b: paying the megamorph cost to turn face up puts the counter.
#[test]
fn a_paid_megamorph_turn_up_puts_its_counter() {
    let outcome = cast_face_down_and_flip("Megamorph {2}", 5);
    assert_eq!(outcome.face_down, Some(false), "{:?}", outcome.prompts);
    assert_eq!(
        outcome.counters,
        Some(1),
        "CR 702.37b: the paid megamorph turn-up places exactly one +1/+1 \
         counter — prompts seen: {:?}",
        outcome.prompts
    );
}

/// CR 702.37: a plain morph turn-up has no counter rider.
#[test]
fn a_paid_morph_turn_up_puts_no_counter() {
    let outcome = cast_face_down_and_flip("Morph {2}", 5);
    assert_eq!(outcome.face_down, Some(false), "{:?}", outcome.prompts);
    assert_eq!(
        outcome.counters,
        Some(0),
        "CR 702.37: a plain morph turn-up places nothing — prompts seen: {:?}",
        outcome.prompts
    );
}

/// CR 702.37b + CR 616.1: a megamorph creature with an ADDITIONAL printed
/// "As … is turned face up" counter replacement — the synthesized rider rides
/// the same pipeline, so the two order together and BOTH apply. The paid flip
/// pauses on the ordering prompt; answering it completes the flip with
/// 1 (rider) + 5 (printed) counters.
#[test]
fn the_rider_orders_with_a_printed_turn_up_replacement() {
    let outcome = cast_face_down_and_flip(
        "Megamorph {2}\nAs this creature is turned face up, put five +1/+1 counters on it.",
        5,
    );
    assert_eq!(outcome.face_down, Some(false), "{:?}", outcome.prompts);
    assert_eq!(
        outcome.counters,
        Some(6),
        "CR 702.37b + CR 616.1: both as-turned-face-up replacements apply \
         (1 + 5) — prompts seen: {:?}",
        outcome.prompts
    );
}

/// CR 702.37b: the rider fires only "if its megamorph cost was paid to turn
/// it face up" — an EFFECT that turns the creature face up publishes no
/// payment fact, so no counter.
#[test]
fn an_effect_driven_turn_up_puts_no_counter() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let creature = scenario
        .add_spell_to_hand_from_oracle(P0, "Hidden Test Creature", false, "Megamorph {2}")
        .as_creature()
        .id();
    let opener = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Test Opener",
            false,
            "Turn target face-down creature face up.",
        )
        .id();
    scenario.with_mana_pool(
        P0,
        (0..6)
            .map(|_| {
                engine::types::mana::ManaUnit::new(
                    engine::types::mana::ManaType::Green,
                    engine::types::identifiers::ObjectId(0),
                    false,
                    vec![],
                )
            })
            .collect(),
    );

    let mut runner = scenario.build();
    runner
        .cast(creature)
        .alternative_cast(AlternativeCastDecision::Alternative)
        .commit();
    let mut prompts = Vec::new();
    for _ in 0..40 {
        match runner.state().waiting_for.clone() {
            WaitingFor::Priority { .. } if runner.state().stack.is_empty() => break,
            WaitingFor::Priority { .. } => {
                if runner.act(GameAction::PassPriority).is_err() {
                    break;
                }
            }
            other => {
                prompts.push(format!("PROMPT: {}", other.variant_name()));
                break;
            }
        }
    }
    let object_id = runner
        .state()
        .objects
        .values()
        .find(|o| o.zone == Zone::Battlefield && o.face_down)
        .map(|o| o.id)
        .expect("a face-down 2/2 must be on the battlefield");

    runner.cast(opener).commit();
    let mut settled = false;
    for _ in 0..40 {
        match runner.state().waiting_for.clone() {
            WaitingFor::Priority { .. } if runner.state().stack.is_empty() => {
                settled = true;
                break;
            }
            WaitingFor::Priority { .. } => {
                if runner.act(GameAction::PassPriority).is_err() {
                    break;
                }
            }
            WaitingFor::TargetSelection { target_slots, .. } => {
                let targets: Vec<_> = target_slots
                    .iter()
                    .filter_map(|slot| slot.legal_targets.first().cloned())
                    .collect();
                if runner.act(GameAction::SelectTargets { targets }).is_err() {
                    break;
                }
            }
            other => {
                prompts.push(format!("PROMPT: {}", other.variant_name()));
                break;
            }
        }
    }
    assert!(
        settled,
        "the opener must resolve — prompts seen: {prompts:?}"
    );

    let obj = &runner.state().objects[&object_id];
    assert!(!obj.face_down, "the effect turned it face up");
    assert_eq!(
        obj.counters
            .get(&CounterType::Plus1Plus1)
            .copied()
            .unwrap_or(0),
        0,
        "CR 702.37b: no megamorph cost was paid, so no counter — prompts: {prompts:?}"
    );
}
