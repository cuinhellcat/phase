//! Issue #3271 — Search for Tomorrow: accepting the suspend last-counter "you
//! may cast it" prompt must put the spell on the stack (CR 702.62a + CR 608.2g).
//!
//! https://github.com/phase-rs/phase/issues/3271
//!
//! Reported bug: removing the last time counter prompts "cast it?", but
//! accepting does nothing — the sorcery never reaches the stack.

use engine::ai_support::legal_actions;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::Effect;
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::events::GameEvent;
use engine::types::game_state::{CastPaymentMode, PersistedGameState, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const SEARCH_FOR_TOMORROW_ORACLE: &str = "Search your library for a basic land card, put it onto the battlefield, then shuffle.\n\
Suspend 2—{G} (Rather than cast this card from your hand, you may pay {G} and exile it with two time counters on it. At the beginning of your upkeep, remove a time counter. When the last is removed, you may cast it without paying its mana cost.)";

fn suspend_ability_index(state: &engine::types::game_state::GameState, card: ObjectId) -> usize {
    legal_actions(state)
        .into_iter()
        .find_map(|action| match action {
            GameAction::ActivateAbility {
                source_id,
                ability_index,
                ..
            } if source_id == card => Some(ability_index),
            _ => None,
        })
        .expect("Search for Tomorrow must offer its Suspend activation from hand")
}

fn time_counters(state: &engine::types::game_state::GameState, card: ObjectId) -> u32 {
    state
        .objects
        .get(&card)
        .and_then(|o| o.counters.get(&CounterType::Time).copied())
        .unwrap_or(0)
}

fn last_counter_cast_uses_during_resolution(
    state: &engine::types::game_state::GameState,
    card: ObjectId,
) -> bool {
    state.objects[&card]
        .trigger_definitions
        .iter_unchecked()
        .find(|t| {
            matches!(
                t.definition.mode,
                engine::types::triggers::TriggerMode::CounterRemoved
            )
        })
        .and_then(|t| t.definition.execute.as_ref())
        .is_some_and(|execute| {
            matches!(
                execute.effect.as_ref(),
                Effect::CastFromZone {
                    driver,
                    without_paying_mana_cost: true,
                    ..
                } if driver.is_during_resolution()
            )
        })
}

/// Drive the turn structure until the suspend last-counter optional cast prompt
/// surfaces, draining the stack and auto-passing combat steps as needed.
fn drive_until_suspend_cast_prompt(
    runner: &mut engine::game::scenario::GameRunner,
) -> Result<(), String> {
    for _ in 0..300 {
        match runner.state().waiting_for.clone() {
            WaitingFor::OptionalEffectChoice { .. } => return Ok(()),
            WaitingFor::Priority { .. } => {
                if !runner.state().stack.is_empty() {
                    runner
                        .act(GameAction::PassPriority)
                        .map_err(|e| format!("PassPriority (stack drain): {e:?}"))?;
                    continue;
                }
                if runner.state().phase == Phase::DeclareAttackers {
                    runner
                        .act(GameAction::DeclareAttackers {
                            attacks: vec![],
                            bands: vec![],
                        })
                        .map_err(|e| format!("DeclareAttackers: {e:?}"))?;
                } else if runner.state().phase == Phase::DeclareBlockers {
                    runner
                        .act(GameAction::DeclareBlockers {
                            assignments: vec![],
                        })
                        .map_err(|e| format!("DeclareBlockers: {e:?}"))?;
                } else {
                    runner
                        .act(GameAction::PassPriority)
                        .map_err(|e| format!("PassPriority: {e:?}"))?;
                    runner
                        .act(GameAction::PassPriority)
                        .map_err(|e| format!("PassPriority (second): {e:?}"))?;
                }
            }
            WaitingFor::DeclareAttackers { .. } => {
                runner
                    .act(GameAction::DeclareAttackers {
                        attacks: vec![],
                        bands: vec![],
                    })
                    .map_err(|e| format!("DeclareAttackers: {e:?}"))?;
            }
            WaitingFor::DeclareBlockers { .. } => {
                runner
                    .act(GameAction::DeclareBlockers {
                        assignments: vec![],
                    })
                    .map_err(|e| format!("DeclareBlockers: {e:?}"))?;
            }
            other => {
                return Err(format!("unexpected waiting_for: {other:?}"));
            }
        }
    }
    Err("timed out waiting for suspend last-counter cast prompt".to_string())
}

#[test]
fn search_for_tomorrow_last_counter_accept_casts_onto_stack() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_library_top(P0, &["Plains", "Plains", "Plains"]);
    scenario.add_basic_land(P0, ManaColor::Green);
    let search = scenario
        .add_spell_to_hand_from_oracle(P0, "Search for Tomorrow", false, SEARCH_FOR_TOMORROW_ORACLE)
        .id();

    let mut runner = scenario.build();

    assert!(
        last_counter_cast_uses_during_resolution(runner.state(), search),
        "parser must synthesize suspend last-counter CastFromZone with DuringResolution driver"
    );

    let suspend_idx = suspend_ability_index(runner.state(), search);
    runner
        .act(hand_action(search, suspend_idx, CastPaymentMode::Auto))
        .unwrap();
    assert!(runner.state().stack.is_empty());

    assert_eq!(runner.state().objects[&search].zone, Zone::Exile);
    assert_eq!(time_counters(runner.state(), search), 2);

    // Fast-forward to one counter remaining, then drive a single upkeep so the
    // last counter is removed and the CounterRemoved cast trigger fires.
    {
        let obj = runner.state_mut().objects.get_mut(&search).unwrap();
        obj.counters.insert(CounterType::Time, 1);
    }
    runner.state_mut().turn_number = 2;
    runner.state_mut().phase = Phase::Untap;
    runner.state_mut().active_player = P0;
    runner.state_mut().priority_player = P0;
    runner.state_mut().waiting_for = WaitingFor::Priority { player: P0 };

    drive_until_suspend_cast_prompt(&mut runner).expect("must reach suspend cast prompt");
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::OptionalEffectChoice { player, .. } if player == P0
        ),
        "expected optional cast prompt for P0, got {:?}",
        runner.state().waiting_for
    );

    runner
        .act(GameAction::DecideOptionalEffect { accept: true })
        .expect("accept suspend last-counter cast");

    assert_eq!(
        runner.state().objects[&search].zone,
        Zone::Stack,
        "accepting must cast Search for Tomorrow during resolution (CR 608.2g); \
         zone = {:?}, waiting_for = {:?}",
        runner.state().objects[&search].zone,
        runner.state().waiting_for,
    );
    assert_eq!(runner.state().stack.len(), 1);
}

const GARGADON_ORACLE: &str = "Suspend 10—{R}\nSacrifice an artifact, creature, or land: Remove a time counter from this card. Activate only if this card is suspended.";

#[test]
fn greater_gargadon_suspends_immediately_from_hand() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mountain = scenario.add_basic_land(P0, ManaColor::Red);
    let gargadon = gargadon_in_hand(&mut scenario);
    let mut runner = scenario.build();
    let index = suspend_ability_index(runner.state(), gargadon);
    let result = runner
        .act(hand_action(gargadon, index, CastPaymentMode::Auto))
        .unwrap();
    assert_eq!(runner.state().objects[&gargadon].zone, Zone::Exile);
    assert_eq!(
        time_counters(runner.state(), gargadon),
        10,
        "time counters must arrive during the hand action"
    );
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, GameEvent::ZoneChanged {
        object_id, from: Some(Zone::Hand), to: Zone::Exile, .. } if *object_id == gargadon)));
    assert!(result.events.iter().any(|event| matches!(event, GameEvent::CounterAdded {
        object_id, counter_type: CounterType::Time, count: 10, actor: P0 } if *object_id == gargadon)));
    assert!(!result.events.iter().any(|event| matches!(event, GameEvent::SpellCast {object_id, ..} | GameEvent::AbilityActivated {source_id:object_id,..} if *object_id == gargadon)));
    assert!(runner.state().priority_passes.is_empty());
    assert!(!runner.state().objects[&gargadon].face_down);
    assert!(runner.state().objects[&mountain].tapped);
    assert!(runner.state().stack.is_empty());
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { player: P0 }
    ));
}

const WHEEL_OF_FATE: &str = "Suspend 4—{1}{R} (Rather than cast this card from your hand, pay {1}{R} and exile it with four time counters on it. At the beginning of your upkeep, remove a time counter. When the last is removed, you may cast it without paying its mana cost.)\nEach player discards their hand, then draws seven cards.";
const THE_WAR_DOCTOR: &str = "Whenever one or more other permanents phase out and whenever one or more other cards are put into exile from anywhere, put a time counter on The War Doctor.\nWhenever The War Doctor attacks, it deals damage equal to the number of time counters on it to any target. If a creature dealt damage this way would die this turn, exile it instead.";
const LOTUS_BLOOM: &str = "Suspend 3—{0} (Rather than cast this card from your hand, pay {0} and exile it with three time counters on it. At the beginning of your upkeep, remove a time counter. When the last is removed, you may cast it without paying its mana cost.)\n{T}, Sacrifice this artifact: Add three mana of any one color.";
const DWARVEN_RUINS: &str =
    "This land enters tapped.\n{T}: Add {R}.\n{T}, Sacrifice this land: Add {R}{R}.";
const REST_IN_PEACE: &str = "When this enchantment enters, exile all graveyards.\nIf a card or token would be put into a graveyard from anywhere, exile it instead.";
const LEYLINE_OF_THE_VOID: &str = "If this card is in your opening hand, you may begin the game with it on the battlefield.\nIf a card would be put into an opponent's graveyard from anywhere, exile it instead.";
const ANCIENT_ZIGGURAT: &str =
    "{T}: Add one mana of any color. Spend this mana only to cast a creature spell.";
const SUNKEN_CITADEL: &str = "This land enters tapped. As it enters, choose a color.\n{T}: Add one mana of the chosen color.\n{T}: Add two mana of the chosen color. Spend this mana only to activate abilities of land sources.";

fn gargadon_in_hand(s: &mut GameScenario) -> ObjectId {
    s.add_creature_to_hand_from_oracle(P0, "Greater Gargadon", 9, 7, GARGADON_ORACLE)
        .with_mana_cost(ManaCost::Cost {
            generic: 9,
            shards: vec![ManaCostShard::Red],
        })
        .id()
}

fn hand_action(card: ObjectId, index: usize, payment_mode: CastPaymentMode) -> GameAction {
    GameAction::ActivateAbility {
        source_id: card,
        ability_index: index,
        payment_mode,
    }
}

fn mana_source(r: &GameRunner, id: ObjectId, color: ManaType) -> GameAction {
    let (_, _, by_object) = engine::ai_support::legal_actions_full(r.state());
    by_object.get(&id).into_iter().flatten().find(|a|
        matches!(a, GameAction::TapLandForMana { selection } if selection.mana_type == color)).expect("select a legal mana source").clone()
}

fn restore(data: &str) -> GameRunner {
    let saved: PersistedGameState = serde_json::from_str(data).expect("production save decoder");
    GameRunner::from_state(saved.into_game_state().expect("checked restore"))
}

#[test]
fn suspend_zero_price_and_normal_cast_have_independent_offers() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let lotus = s
        .add_artifact_to_hand_from_oracle(P0, "Lotus Bloom", LOTUS_BLOOM)
        .with_mana_cost(ManaCost::NoCost)
        .id();
    let mut r = s.build();
    let ix = suspend_ability_index(r.state(), lotus);
    assert!(!legal_actions(r.state())
        .iter()
        .any(|a| matches!(a, GameAction::CastSpell {object_id,..} if *object_id == lotus)));
    r.act(hand_action(lotus, ix, CastPaymentMode::Auto))
        .unwrap();
    assert_eq!(time_counters(r.state(), lotus), 3);
    assert_eq!(r.state().objects[&lotus].zone, Zone::Exile);
    assert!(!r.state().objects[&lotus].face_down);
    assert!(r.state().stack.is_empty() && r.state().pending_cast.is_none());

    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let card = gargadon_in_hand(&mut s);
    for _ in 0..10 {
        s.add_basic_land(P0, ManaColor::Red);
    }
    let mut r = s.build();
    let ix = suspend_ability_index(r.state(), card);
    let actions = legal_actions(r.state());
    assert!(actions
        .iter()
        .any(|a| matches!(a,GameAction::CastSpell {object_id,..} if *object_id == card)));
    let other = r.state().objects[&card]
        .abilities
        .iter()
        .position(|a| a.activation_zone != Some(Zone::Hand))
        .unwrap();
    assert_ne!(other, ix);
    assert!(r
        .act(hand_action(card, other, CastPaymentMode::Auto))
        .is_err());
    assert_eq!(r.state().objects[&card].zone, Zone::Hand);
    r.act(hand_action(card, ix, CastPaymentMode::Auto)).unwrap();
    assert_eq!(time_counters(r.state(), card), 10);
}

#[test]
fn suspend_exile_collects_the_war_doctors_real_trigger() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_basic_land(P0, ManaColor::Red);
    let doctor = s
        .add_creature_from_oracle(P0, "The War Doctor", 3, 5, THE_WAR_DOCTOR)
        .id();
    let card = gargadon_in_hand(&mut s);
    let mut r = s.build();
    let ix = suspend_ability_index(r.state(), card);
    r.act(hand_action(card, ix, CastPaymentMode::Auto)).unwrap();
    assert_eq!(time_counters(r.state(), card), 10);
    assert_eq!(r.state().stack.len(), 1);
    assert_eq!(time_counters(r.state(), doctor), 0);
    r.resolve_top();
    assert_eq!(time_counters(r.state(), doctor), 1);
    assert_eq!(time_counters(r.state(), card), 10);
    assert!(r.state().stack.is_empty());
}

#[test]
fn suspend_manual_payment_pins_one_source_and_cancels_unpaid() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let first = s.add_basic_land(P0, ManaColor::Red);
    let second = s.add_basic_land(P0, ManaColor::Red);
    let card = gargadon_in_hand(&mut s);
    let mut r = s.build();
    let ix = suspend_ability_index(r.state(), card);
    r.act(hand_action(card, ix, CastPaymentMode::Manual))
        .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ManaPayment { player: P0, .. }
    ));
    let opponent_view = engine::game::visibility::filter_state_for_viewer(r.state(), P1);
    assert!(
        opponent_view.pending_cast.is_none(),
        "the opponent cannot inspect the hand card being paid for"
    );
    assert!(
        engine::game::visibility::filter_state_for_viewer(r.state(), P0)
            .pending_cast
            .is_some()
    );
    assert_eq!(r.state().objects[&card].zone, Zone::Hand);
    assert!(r.state().stack.is_empty());
    let mut canceled = GameRunner::from_state(r.state().clone());
    canceled.act(GameAction::CancelCast).unwrap();
    assert_eq!(canceled.state().objects[&card].zone, Zone::Hand);
    assert!(canceled.state().pending_cast.is_none());
    assert!(!canceled.state().objects[&second].tapped);
    assert!(r.act(hand_action(card, ix, CastPaymentMode::Auto)).is_err());
    r.act(mana_source(&r, second, ManaType::Red)).unwrap();
    let pip = r.state().players[0]
        .mana_pool
        .units()
        .find(|u| u.source_id == second)
        .unwrap()
        .pip_id;
    r.act(GameAction::SpendPoolMana { pip_id: pip }).unwrap();
    // Within ManaPayment this wire action confirms the selected payment.
    r.act(GameAction::PassPriority).unwrap();
    assert_eq!(r.state().objects[&card].zone, Zone::Exile);
    assert_eq!(time_counters(r.state(), card), 10);
    assert!(r.state().stack.is_empty() && r.state().pending_cast.is_none());
    assert!(r.state().objects[&second].tapped && !r.state().objects[&first].tapped);
    assert_eq!(r.state().players[0].mana_pool.total(), 0);
}

#[test]
fn suspend_restores_a_pre_special_action_manual_payment() {
    // Captured from the pre-special-action engine immediately after announcing
    // Greater Gargadon's indexed hand ability in Manual payment mode.
    let mut r = restore(include_str!(
        "../fixtures/activation_cost/greater_gargadon_pre_special_action_manual.json"
    ));
    let card = ObjectId(2);
    let mountain = r
        .state()
        .battlefield
        .iter()
        .copied()
        .find(|id| r.state().objects[id].name == "Mountain")
        .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ManaPayment { player: P0, .. }
    ));
    r.act(mana_source(&r, mountain, ManaType::Red)).unwrap();
    let events = r.act(GameAction::PassPriority).unwrap().events;
    assert_eq!(time_counters(r.state(), card), 10);
    assert_eq!(r.state().objects[&card].zone, Zone::Exile);
    assert!(r.state().stack.is_empty() && r.state().pending_cast.is_none());
    assert!(r.state().objects[&mountain].tapped);
    assert_eq!(r.state().players[0].mana_pool.total(), 0);
    assert!(!events
        .iter()
        .any(|e| matches!(e, GameEvent::AbilityActivated { .. })));
}

#[test]
fn suspend_stale_manual_payment_cannot_exile_the_new_hand_incarnation() {
    let mut r = restore(include_str!(
        "../fixtures/activation_cost/greater_gargadon_pre_special_action_manual.json"
    ));
    let card = ObjectId(2);
    let mountain = r
        .state()
        .battlefield
        .iter()
        .copied()
        .find(|id| r.state().objects[id].name == "Mountain")
        .unwrap();
    r.act(mana_source(&r, mountain, ManaType::Red)).unwrap();
    for zone in [Zone::Exile, Zone::Hand] {
        engine::game::zones::move_to_zone(r.state_mut(), card, zone, &mut Vec::new());
    }
    assert!(r.act(GameAction::PassPriority).is_err());
    assert_eq!(r.state().objects[&card].zone, Zone::Hand);
    assert_eq!(time_counters(r.state(), card), 0);
    assert_eq!(r.state().players[0].mana_pool.total(), 1);
}

#[test]
fn suspend_auto_sacrifice_choice_restores_and_manual_policy_spares_the_source() {
    for mode in [
        CastPaymentMode::Auto,
        CastPaymentMode::AutoExceptSacrificialMana,
    ] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        let ruins = s
            .add_land_from_oracle(P0, "Dwarven Ruins", DWARVEN_RUINS)
            .id();
        s.add_enchantment_from_oracle(P1, "Rest in Peace", REST_IN_PEACE);
        s.add_enchantment_from_oracle(P1, "Leyline of the Void", LEYLINE_OF_THE_VOID);
        let wheel = s
            .add_spell_to_hand_from_oracle(P0, "Wheel of Fate", false, WHEEL_OF_FATE)
            .with_mana_cost(ManaCost::NoCost)
            .id();
        let mut r = s.build();
        r.state_mut().objects.get_mut(&ruins).unwrap().tapped = false;
        let ix = suspend_ability_index(r.state(), wheel);
        r.act(hand_action(wheel, ix, mode)).unwrap();
        assert_eq!(r.state().objects[&wheel].zone, Zone::Hand);
        assert!(r.state().stack.is_empty());
        if mode == CastPaymentMode::AutoExceptSacrificialMana {
            assert!(matches!(
                r.state().waiting_for,
                WaitingFor::ManaPayment { .. } | WaitingFor::ManaSourceSelection { .. }
            ));
            assert!(!r.state().objects[&ruins].tapped);
            continue;
        }
        let WaitingFor::ReplacementChoice {
            player: P0,
            candidates,
            ..
        } = &r.state().waiting_for
        else {
            panic!("expected real replacement choice")
        };
        let mut names: Vec<_> = candidates.iter().map(|s| s.source_name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["Leyline of the Void", "Rest in Peace"]);
        let serialized =
            serde_json::to_string(&PersistedGameState::capture(r.state().clone())).unwrap();
        let mut r = restore(&serialized);
        r.act(GameAction::ChooseReplacement { index: 0 }).unwrap();
        assert_eq!(r.state().objects[&ruins].zone, Zone::Exile);
        assert_eq!(r.state().objects[&wheel].zone, Zone::Exile);
        assert!(!r.state().objects[&wheel].face_down);
        assert_eq!(time_counters(r.state(), wheel), 4);
        assert!(r.state().stack.is_empty() && r.state().pending_cast.is_none());
        assert_eq!(r.state().players[0].mana_pool.total(), 0);
    }
}

#[test]
fn suspend_cannot_spend_creature_only_mana_although_a_creature_can() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let ziggurat = s
        .add_land_from_oracle(P0, "Ancient Ziggurat", ANCIENT_ZIGGURAT)
        .id();
    let card = gargadon_in_hand(&mut s);
    let goblin = s
        .add_creature_to_hand_from_oracle(
            P0,
            "Raging Goblin",
            1,
            1,
            "Haste (This creature can attack and {T} as soon as it comes under your control.)",
        )
        .with_mana_cost(ManaCost::Cost {
            generic: 0,
            shards: vec![ManaCostShard::Red],
        })
        .id();
    let mut r = s.build();
    let ix = r.state().objects[&card]
        .abilities
        .iter()
        .position(|d| d.activation_zone == Some(Zone::Hand))
        .unwrap();
    assert!(!legal_actions(r.state()).contains(&hand_action(card, ix, CastPaymentMode::Auto)));
    assert!(r.act(hand_action(card, ix, CastPaymentMode::Auto)).is_err());
    assert!(!r.state().objects[&ziggurat].tapped);
    let mana_index = r.state().objects[&ziggurat]
        .abilities
        .iter()
        .position(engine::game::mana_abilities::is_mana_ability)
        .expect("Ancient Ziggurat's printed mana ability");
    r.act(hand_action(ziggurat, mana_index, CastPaymentMode::Auto))
        .unwrap();
    if matches!(r.state().waiting_for, WaitingFor::ChooseManaColor { .. }) {
        r.act(GameAction::ChooseManaColor {
            choice: engine::types::game_state::ManaChoice::SingleColor(ManaType::Red),
            count: 1,
        })
        .unwrap();
    }
    assert!(r.state().players[0]
        .mana_pool
        .units()
        .any(|u| u.source_id == ziggurat
            && u.color == ManaType::Red
            && !u.restrictions.is_empty()));
    assert_eq!(r.state().players[0].mana_pool.total(), 1);
    assert!(r.act(hand_action(card, ix, CastPaymentMode::Auto)).is_err());
    r.cast(goblin).resolve();
    assert_eq!(r.state().objects[&goblin].zone, Zone::Battlefield);
    assert_eq!(r.state().players[0].mana_pool.total(), 0);
}

#[test]
fn suspend_cannot_spend_or_pin_land_activation_only_mana() {
    use engine::types::ability::ChosenAttribute;
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let citadel = s
        .add_land_from_oracle(P0, "Sunken Citadel", SUNKEN_CITADEL)
        .id();
    let encampment=s.add_land_from_oracle(P0,"Ghitu Encampment","This land enters tapped.\n{T}: Add {R}.\n{1}{R}: This land becomes a 2/1 red Warrior creature with first strike until end of turn. It's still a land. (It deals combat damage before creatures without first strike.)").id();
    let mountain = s.add_basic_land(P0, ManaColor::Red);
    let card = gargadon_in_hand(&mut s);
    let mut r = s.build();
    r.state_mut().objects.get_mut(&encampment).unwrap().tapped = true;
    let land = r.state_mut().objects.get_mut(&citadel).unwrap();
    land.tapped = false;
    land.chosen_attributes
        .push(ChosenAttribute::Color(ManaColor::Red));
    let selected = engine::game::mana_sources::activatable_mana_source_selections(r.state(), P0)
        .into_iter()
        .find(|sel| {
            sel.source.object_id == citadel
                && sel.mana_type == ManaType::Red
                && !sel.restrictions.is_empty()
        })
        .unwrap();
    r.act(GameAction::TapLandForMana {
        selection: selected,
    })
    .unwrap();
    assert_eq!(r.state().players[0].mana_pool.total(), 2);
    let ix = suspend_ability_index(r.state(), card);
    let mut only_citadel = GameRunner::from_state(r.state().clone());
    only_citadel
        .state_mut()
        .objects
        .get_mut(&mountain)
        .unwrap()
        .tapped = true;
    assert!(!legal_actions(only_citadel.state()).contains(&hand_action(
        card,
        ix,
        CastPaymentMode::Auto
    )));
    assert!(only_citadel
        .act(hand_action(card, ix, CastPaymentMode::Auto))
        .is_err());
    let land_ability = only_citadel.state().objects[&encampment]
        .abilities
        .iter()
        .position(|a| !engine::game::mana_abilities::is_mana_ability(a))
        .unwrap();
    only_citadel
        .act(hand_action(encampment, land_ability, CastPaymentMode::Auto))
        .unwrap();
    only_citadel.resolve_top();
    assert_eq!(only_citadel.state().players[0].mana_pool.total(), 0);
    assert!(only_citadel.state().objects[&encampment]
        .card_types
        .core_types
        .contains(&engine::types::card_type::CoreType::Creature));
    r.act(hand_action(card, ix, CastPaymentMode::Manual))
        .unwrap();
    let restricted = r.state().players[0]
        .mana_pool
        .units()
        .next()
        .unwrap()
        .pip_id;
    assert!(r
        .act(GameAction::SpendPoolMana { pip_id: restricted })
        .is_err());
    r.act(mana_source(&r, mountain, ManaType::Red)).unwrap();
    r.act(GameAction::PassPriority).unwrap();
    assert_eq!(time_counters(r.state(), card), 10);
    assert_eq!(r.state().players[0].mana_pool.total(), 2);
}

#[test]
fn suspend_legacy_lookalike_and_ordinary_manual_activation_stay_distinct() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_basic_land(P0, ManaColor::Red);
    let card = gargadon_in_hand(&mut s);
    let r = s.build();
    for change_count in [false, true] {
        let mut r = GameRunner::from_state(r.state().clone());
        let ix = suspend_ability_index(r.state(), card);
        let obj = r.state_mut().objects.get_mut(&card).unwrap();
        let def = &mut std::sync::Arc::make_mut(&mut obj.abilities)[ix];
        def.ability_tag = None;
        if change_count {
            let Effect::PutCounter { count, .. } = def.effect.as_mut() else {
                panic!("suspend descriptor")
            };
            *count = engine::types::ability::QuantityExpr::Fixed { value: 9 };
        }
        obj.base_abilities = obj.abilities.clone();
        r.act(hand_action(card, ix, CastPaymentMode::Auto)).unwrap();
        if change_count {
            assert_eq!(time_counters(r.state(), card), 0);
            assert_eq!(r.state().stack.len(), 1);
            r.resolve_top();
            assert_eq!(time_counters(r.state(), card), 9);
        } else {
            assert!(r.state().stack.is_empty());
            assert_eq!(time_counters(r.state(), card), 10);
        }
    }

    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let mountain = s.add_basic_land(P0, ManaColor::Red);
    let dragon = s
        .add_creature_from_oracle(
            P0,
            "Shivan Dragon",
            5,
            5,
            "Flying\n{R}: This creature gets +1/+0 until end of turn.",
        )
        .id();
    let mut r = s.build();
    let ix = r.state().objects[&dragon]
        .abilities
        .iter()
        .position(|d| d.cost.is_some())
        .unwrap();
    r.act(hand_action(dragon, ix, CastPaymentMode::Manual))
        .unwrap();
    let saved = serde_json::to_string(&PersistedGameState::capture(r.state().clone())).unwrap();
    let mut r = restore(&saved);
    r.act(mana_source(&r, mountain, ManaType::Red)).unwrap();
    r.act(GameAction::PassPriority).unwrap();
    assert_eq!(r.state().stack.len(), 1);
    r.resolve_top();
    assert_eq!(r.state().objects[&dragon].power, Some(6));
}

#[test]
fn suspend_price_ignores_a_real_reduction_to_activated_abilities() {
    const ZIRDA: &str = "Companion — Each permanent card in your starting deck has an activated ability. (If this card is your chosen companion, you may put it into your hand from outside the game for {3} as a sorcery.)\nAbilities you activate that aren't mana abilities cost {2} less to activate. This effect can't reduce the mana in that cost to less than one mana.\n{1}, {T}: Target creature can't block this turn.";
    for has_second_land in [false, true] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        s.add_creature_from_oracle(P0, "Zirda, the Dawnwaker", 3, 3, ZIRDA);
        let tome = s
            .add_artifact_from_oracle(
                P0,
                "Jalum Tome",
                "{2}, {T}: Draw a card, then discard a card.",
            )
            .id();
        let mountain = s.add_basic_land(P0, ManaColor::Red);
        if has_second_land {
            s.add_basic_land(P0, ManaColor::Green);
        }
        let card = s
            .add_spell_to_hand_from_oracle(P0, "Wheel of Fate", false, WHEEL_OF_FATE)
            .with_mana_cost(ManaCost::NoCost)
            .id();
        let mut r = s.build();
        let index = r.state().objects[&card]
            .abilities
            .iter()
            .position(|d| d.activation_zone == Some(Zone::Hand))
            .unwrap();
        let action = hand_action(card, index, CastPaymentMode::Auto);
        assert_eq!(legal_actions(r.state()).contains(&action), has_second_land);
        if has_second_land {
            r.act(action).unwrap();
            assert_eq!(time_counters(r.state(), card), 4);
            assert!(r.state().stack.is_empty());
        } else {
            assert!(r.act(action).is_err());
            assert!(!r.state().objects[&mountain].tapped);
            let index = r.state().objects[&tome]
                .abilities
                .iter()
                .position(|d| d.cost.is_some())
                .unwrap();
            r.act(hand_action(tome, index, CastPaymentMode::Auto))
                .unwrap();
            assert!(r.state().objects[&mountain].tapped && r.state().objects[&tome].tapped);
            assert_eq!(r.state().stack.len(), 1);
        }
    }
}

#[test]
fn suspend_safe_auto_tap_resolves_the_lands_mana_choice_before_spending() {
    const PLANE: &str="If an effect would create one or more tokens, it creates twice that many of those tokens instead.\nIf an effect would put one or more counters on a permanent, it puts twice that many of those counters on that permanent instead.\nWhenever chaos ensues, until end of turn, whenever you tap a land for mana, add one mana of any type that land produced.";
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let plane = s
        .add_land_from_oracle(P0, "Selesnya Loft Gardens", PLANE)
        .id();
    let chamber=s.add_land_from_oracle(P0,"Simic Growth Chamber","This land enters tapped.\nWhen this land enters, return a land you control to its owner's hand.\n{T}: Add {G}{U}.").id();
    let card = s
        .add_spell_to_hand_from_oracle(P0, "Search for Tomorrow", false, SEARCH_FOR_TOMORROW_ORACLE)
        .id();
    let mut r = s.build();
    r.state_mut().objects.get_mut(&chamber).unwrap().tapped = false;
    engine::game::triggers::process_triggers(
        r.state_mut(),
        &[GameEvent::ChaosEnsued { plane_id: plane }],
    );
    while r.state().delayed_triggers.is_empty() && !r.state().stack.is_empty() {
        r.act(GameAction::PassPriority).unwrap();
    }
    assert_eq!(r.state().delayed_triggers.len(), 1);
    let ix = suspend_ability_index(r.state(), card);
    r.act(hand_action(
        card,
        ix,
        CastPaymentMode::AutoExceptSacrificialMana,
    ))
    .unwrap();
    assert_eq!(r.state().objects[&card].zone, Zone::Hand);
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ChooseManaColor { .. }
    ));
    assert_eq!(r.state().players[0].mana_pool.total(), 2);
    r.act(GameAction::ChooseManaColor {
        choice: engine::types::game_state::ManaChoice::SingleColor(ManaType::Blue),
        count: 1,
    })
    .unwrap();
    assert_eq!(time_counters(r.state(), card), 2);
    assert_eq!(
        r.state().players[0].mana_pool.count_color(ManaType::Blue),
        2
    );
    assert!(r.state().stack.is_empty() && r.state().pending_cast.is_none());
}
