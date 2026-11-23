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
use engine::types::game_state::{CastPaymentMode, PersistedGameState, StackEntryKind, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const SEARCH_FOR_TOMORROW_ORACLE: &str = "Search your library for a basic land card, put it onto the battlefield, then shuffle.\n\
Suspend 2—{G} (Rather than cast this card from your hand, you may pay {G} and exile it with two time counters on it. At the beginning of your upkeep, remove a time counter. When the last is removed, you may cast it without paying its mana cost.)";
const KILLIAN_ORACLE: &str = "Lifelink\nMenace (This creature can't be blocked except by two or more creatures.)\nSpells you cast that target a creature cost {2} less to cast.";
const GRAFTED_IDENTITY: &str = "As an additional cost to cast this spell, sacrifice a creature.\nEnchant creature\nYou control enchanted creature.\nEnchanted creature gets +1/+1.";

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

fn corpse_payment_board(with_krrik: bool, float_mana: bool) -> (GameRunner, ObjectId, ObjectId) {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain).with_life(P0, 20);
    if with_krrik {
        s.add_creature_from_oracle(P0, "K'rrik, Son of Yawgmoth", 2, 2,
    "({B/P} can be paid with either {B} or 2 life.)\nLifelink\nFor each {B} in a cost, you may pay 2 life rather than pay that mana.\nWhenever you cast a black spell, put a +1/+1 counter on K'rrik.");
    }
    let corpse = s.add_creature_to_hand_from_oracle(P0, "Corpulent Corpse", 3, 3,
    "Fear (This creature can't be blocked except by artifact creatures and/or black creatures.)\nSuspend 5—{B} (Rather than cast this card from your hand, you may pay {B} and exile it with five time counters on it. At the beginning of your upkeep, remove a time counter. When the last is removed, you may cast it without paying its mana cost. It has haste.)")
    .with_mana_cost(ManaCost::Cost {generic: 5, shards: vec![ManaCostShard::Black]}).id();
    let swamp = s.add_basic_land(P0, ManaColor::Black);
    let mut r = s.build();
    if float_mana {
        r.act(mana_source(&r, swamp, ManaType::Black)).unwrap();
    }
    (r, corpse, swamp)
}

#[test]
fn suspend_manual_payment_krrik_offers_black_mana_or_two_life() {
    use engine::types::game_state::{PhyrexianShard, ShardChoice, ShardOptions};
    let confirm = |with_krrik: bool, float_mana: bool| {
        let (mut r, corpse, swamp) = corpse_payment_board(with_krrik, float_mana);
        let ix = suspend_ability_index(r.state(), corpse);
        r.act(hand_action(corpse, ix, CastPaymentMode::Manual))
            .unwrap();
        r.act(GameAction::PassPriority).unwrap();
        (r, corpse, swamp)
    };
    let (ordinary, corpse, _) = confirm(false, true);
    assert_eq!(ordinary.state().objects[&corpse].zone, Zone::Exile);
    assert_eq!(time_counters(ordinary.state(), corpse), 5);
    assert_eq!(ordinary.state().players[0].life, 20);
    assert_eq!(ordinary.state().players[0].mana_pool.total(), 0);

    let (mut unfunded, corpse, swamp) = confirm(true, false);
    assert!(
        matches!(&unfunded.state().waiting_for, WaitingFor::PhyrexianPayment { shards, .. }
        if shards.len() == 1 && shards[0].options == ShardOptions::LifeOnly)
    );
    let before = serde_json::to_value(unfunded.state()).unwrap();
    assert!(unfunded
        .act(GameAction::SubmitPhyrexianChoices {
            choices: vec![ShardChoice::PayMana]
        })
        .is_err());
    assert_eq!(serde_json::to_value(unfunded.state()).unwrap(), before);
    unfunded
        .act(GameAction::SubmitPhyrexianChoices {
            choices: vec![ShardChoice::PayLife],
        })
        .unwrap();
    assert_eq!(unfunded.state().players[0].life, 18);
    assert!(!unfunded.state().objects[&swamp].tapped);
    assert_eq!(time_counters(unfunded.state(), corpse), 5);

    let (r, corpse, _) = confirm(true, true);
    let WaitingFor::PhyrexianPayment {
        player: P0,
        spell_object,
        shards,
    } = &r.state().waiting_for
    else {
        panic!("K'rrik must offer a choice before Corpulent Corpse is suspended")
    };
    assert_eq!(*spell_object, corpse);
    assert_eq!(
        shards.as_slice(),
        &[PhyrexianShard {
            shard_index: 0,
            color: ManaColor::Black,
            options: ShardOptions::ManaOrLife
        }]
    );
    assert_eq!(r.state().objects[&corpse].zone, Zone::Hand);
    assert_eq!(r.state().players[0].life, 20);
    assert_eq!(
        r.state().players[0].mana_pool.count_color(ManaType::Black),
        1
    );
    for (choice, life, black) in [(ShardChoice::PayLife, 18, 1), (ShardChoice::PayMana, 20, 0)] {
        let mut branch = GameRunner::from_state(r.state().clone());
        let result = branch
            .act(GameAction::SubmitPhyrexianChoices {
                choices: vec![choice],
            })
            .unwrap();
        let state = branch.state();
        assert!(!result.events.iter().any(|e| matches!(
            e,
            GameEvent::SpellCast { .. } | GameEvent::AbilityActivated { .. }
        )));
        assert!(matches!(
            state.waiting_for,
            WaitingFor::Priority { player: P0 }
        ));
        assert_eq!(state.players[0].life, life);
        assert_eq!(
            state.players[0].mana_pool.count_color(ManaType::Black),
            black
        );
        assert_eq!(state.objects[&corpse].zone, Zone::Exile);
        assert!(!state.objects[&corpse].face_down);
        assert_eq!(time_counters(state, corpse), 5);
        assert!(state.stack.is_empty() && state.pending_cast.is_none());
    }
}

fn automatic_corpse_payment(mode: CastPaymentMode) {
    use engine::types::game_state::{ShardChoice, ShardOptions};
    for (float, tapped, fund_now, options) in [
        (true, true, false, ShardOptions::ManaOrLife),
        (false, false, true, ShardOptions::ManaOrLife),
        (false, false, false, ShardOptions::LifeOnly),
        (false, true, false, ShardOptions::LifeOnly),
    ] {
        let (mut r, corpse, swamp) = corpse_payment_board(true, float);
        r.state_mut().objects.get_mut(&swamp).unwrap().tapped = tapped;
        let ix = suspend_ability_index(r.state(), corpse);
        r.act(hand_action(corpse, ix, mode)).unwrap();
        if !float {
            assert!(matches!(
                r.state().waiting_for,
                WaitingFor::ManaPayment { player: P0, .. }
            ));
            assert_eq!(r.state().objects[&swamp].tapped, tapped);
            if fund_now {
                r.act(mana_source(&r, swamp, ManaType::Black)).unwrap();
            }
            r.act(GameAction::PassPriority).unwrap();
        }
        assert!(
            matches!(&r.state().waiting_for, WaitingFor::PhyrexianPayment { player: P0, spell_object, shards }
            if *spell_object == corpse && shards.len() == 1 && shards[0].options == options),
            "{mode:?} must present K'rrik's {options:?} election before payment: {:?}",
            r.state().waiting_for
        );
        assert_eq!(r.state().objects[&corpse].zone, Zone::Hand);
        assert_eq!(r.state().objects[&swamp].tapped, tapped || fund_now);
        assert_eq!(r.state().players[0].life, 20);
        assert_eq!(
            r.state().players[0].mana_pool.total(),
            usize::from(float || fund_now)
        );
        let mut cancelled = GameRunner::from_state(r.state().clone());
        cancelled.act(GameAction::CancelCast).unwrap();
        assert_eq!(cancelled.state().objects[&corpse].zone, Zone::Hand);
        assert_eq!(cancelled.state().players[0].life, 20);
        assert!(cancelled.state().pending_cast.is_none());
        for choice in [ShardChoice::PayLife, ShardChoice::PayMana] {
            let mut branch = GameRunner::from_state(r.state().clone());
            let result = branch.act(GameAction::SubmitPhyrexianChoices {
                choices: vec![choice],
            });
            let life = choice == ShardChoice::PayLife;
            if !life && options == ShardOptions::LifeOnly {
                assert!(result.is_err());
                continue;
            }
            result.unwrap();
            let state = branch.state();
            assert_eq!(state.objects[&swamp].tapped, tapped || fund_now);
            assert_eq!(state.objects[&corpse].zone, Zone::Exile);
            assert_eq!(time_counters(state, corpse), 5);
            assert_eq!(state.players[0].life, if life { 18 } else { 20 });
            assert_eq!(
                state.players[0].mana_pool.total(),
                usize::from((float || fund_now) && life)
            );
            assert!(matches!(
                state.waiting_for,
                WaitingFor::Priority { player: P0 }
            ));
            assert!(state.pending_cast.is_none() && state.stack.is_empty());
        }
    }
}

#[test]
fn suspend_auto_krrik_preserves_the_payment_choice() {
    automatic_corpse_payment(CastPaymentMode::Auto);
}

#[test]
fn suspend_auto_except_sacrificial_krrik_preserves_the_payment_choice() {
    automatic_corpse_payment(CastPaymentMode::AutoExceptSacrificialMana);
}

#[test]
fn suspend_manual_payment_teval_only_offers_delve_for_a_spell() {
    use engine::game::interaction::{
        bind_interaction_authority, derive_viewer_interaction, interaction_action_id,
    };
    use engine::game::visibility::filter_state_for_viewer;
    use engine::types::interaction::{
        InteractionOpportunityResponse, InteractionPresentationSurface, InteractionSessionId,
    };
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_creature_from_oracle(P0, "Teval, Arbiter of Virtue", 6, 6,
        "Flying, lifelink\nSpells you cast have delve. (Each card you exile from your graveyard while casting those spells pays for {1}.)\nWhenever you cast a spell, you lose life equal to its mana value.");
    let talisman = s.add_artifact_to_hand_from_oracle(P0, "Sol Talisman",
        "Suspend 3—{1} (Rather than cast this card from your hand, pay {1} and exile it with three time counters on it. At the beginning of your upkeep, remove a time counter. When the last is removed, you may cast it without paying its mana cost.)\n{T}: Add {C}{C}.")
        .with_mana_cost(ManaCost::NoCost).id();
    let ring = s
        .add_artifact_to_hand_from_oracle(P0, "Sol Ring", "{T}: Add {C}{C}.")
        .with_mana_cost(ManaCost::Cost {
            generic: 1,
            shards: vec![],
        })
        .id();
    let swamp = s.add_basic_land(P0, ManaColor::Black);
    let fuel = s.add_land_to_graveyard(P0, "Island").id();
    let mut r = s.build();
    bind_interaction_authority(r.state_mut(), InteractionSessionId("suspend-teval".into()))
        .unwrap();
    let delve = GameAction::TapForConvoke {
        object_id: fuel,
        mana_type: ManaType::Colorless,
    };
    let published = |r: &GameRunner| {
        let view =
            derive_viewer_interaction(r.state(), &filter_state_for_viewer(r.state(), P0), P0);
        let InteractionOpportunityResponse::ExactChoices { choices } =
            &view.opportunities[0].response
        else {
            panic!("expected manual mana payment choices")
        };
        choices.iter().any(|c| c.surfaces.iter().any(|s| matches!(s, InteractionPresentationSurface::Action {action_id: Some(id),..} if *id == interaction_action_id(&delve))))
    };
    let candidate_offered = |r: &GameRunner| {
        engine::ai_support::candidate_actions(r.state())
            .iter()
            .any(|candidate| candidate.action == delve)
    };
    let mut spell = GameRunner::from_state(r.state().clone());
    spell
        .act(GameAction::CastSpell {
            object_id: ring,
            card_id: spell.state().objects[&ring].card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Manual,
        })
        .unwrap();
    assert!(published(&spell));
    assert!(candidate_offered(&spell));
    assert!(legal_actions(spell.state()).contains(&delve));
    spell.act(delve.clone()).unwrap();
    assert_eq!(spell.state().objects[&fuel].zone, Zone::Graveyard);
    spell.act(GameAction::PassPriority).unwrap();
    assert_eq!(spell.state().objects[&fuel].zone, Zone::Exile);
    assert_eq!(spell.state().objects[&ring].zone, Zone::Stack);
    assert!(!spell.state().objects[&swamp].tapped);
    assert_eq!(spell.state().players[0].mana_pool.total(), 0);
    assert_eq!(spell.state().stack.len(), 2);
    let life_before = spell.state().players[0].life;
    spell.resolve_top();
    assert_eq!(spell.state().players[0].life, life_before - 1);
    spell.resolve_top();
    assert_eq!(spell.state().objects[&ring].zone, Zone::Battlefield);

    let ix = suspend_ability_index(r.state(), talisman);
    r.act(hand_action(talisman, ix, CastPaymentMode::Manual))
        .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ManaPayment { player: P0, .. }
    ));
    let viewer_offers_delve = published(&r);
    let legal_offers_delve = legal_actions(r.state()).contains(&delve);
    assert!(!candidate_offered(&r));
    let before = serde_json::to_value(r.state()).unwrap();
    let accepted_delve = r.act(delve).is_ok();
    let state_changed = serde_json::to_value(r.state()).unwrap() != before;
    r.act(mana_source(&r, swamp, ManaType::Black)).unwrap();
    r.act(GameAction::PassPriority).unwrap();
    assert_eq!(r.state().objects[&fuel].zone, Zone::Graveyard);
    assert_eq!(r.state().objects[&talisman].zone, Zone::Exile);
    assert_eq!(time_counters(r.state(), talisman), 3);
    assert!(!r.state().objects[&talisman].face_down);
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::Priority { player: P0 }
    ));
    assert!(r.state().objects[&swamp].tapped);
    assert_eq!(
        (viewer_offers_delve, legal_offers_delve, accepted_delve, state_changed, r.state().players[0].mana_pool.total()),
        (false, false, false, false, 0),
        "suspending must neither offer nor accept delve, and spending real mana must leave no payment marker",
    );
    assert!(r.state().stack.is_empty() && r.state().pending_cast.is_none());
}

fn printed_card(
    scenario: &mut GameScenario,
    player: engine::types::player::PlayerId,
    name: &str,
    zone: Zone,
) -> ObjectId {
    use engine::game::scenario_db::GameScenarioDbExt;
    let db = crate::support::shared_card_db().expect("committed integration card fixture");
    scenario.add_real_card(player, name, zone, db)
}

fn pass_pair(runner: &mut GameRunner) {
    runner.act(GameAction::PassPriority).unwrap();
    runner.act(GameAction::PassPriority).unwrap();
}

fn one_counter_upkeep(runner: &mut GameRunner, card: ObjectId) {
    runner
        .state_mut()
        .objects
        .get_mut(&card)
        .unwrap()
        .counters
        .insert(CounterType::Time, 1);
    runner.state_mut().turn_number = 2;
    runner.state_mut().phase = Phase::Untap;
    runner.state_mut().active_player = P0;
    runner.state_mut().priority_player = P0;
    runner.state_mut().waiting_for = WaitingFor::Priority { player: P0 };
    pass_pair(runner);
    assert_eq!(runner.state().phase, Phase::Upkeep);
    assert_eq!(runner.state().stack.len(), 1);
    assert_eq!(time_counters(runner.state(), card), 1);
}

fn gargadon_at_last_counter(setup: impl FnOnce(&mut GameScenario)) -> (GameRunner, ObjectId) {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_basic_land(P0, ManaColor::Red);
    let card = gargadon_in_hand(&mut s);
    setup(&mut s);
    let mut r = s.build();
    let index = suspend_ability_index(r.state(), card);
    r.act(hand_action(card, index, CastPaymentMode::Auto))
        .unwrap();
    one_counter_upkeep(&mut r, card);
    pass_pair(&mut r);
    assert_eq!(time_counters(r.state(), card), 0);
    assert_eq!(r.state().stack.len(), 1);
    (r, card)
}

#[test]
fn suspend_last_counter_requires_and_charges_the_casting_tax() {
    for can_pay in [false, true] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        s.add_artifact_from_oracle(P1, "Sphere of Resistance", "Spells cost {1} more to cast.");
        let mountain = s.add_basic_land(P0, ManaColor::Red);
        let plains = can_pay.then(|| s.add_basic_land(P0, ManaColor::White));
        let card = gargadon_in_hand(&mut s);
        let mut r = s.build();
        r.act(hand_action(
            card,
            suspend_ability_index(r.state(), card),
            CastPaymentMode::Auto,
        ))
        .unwrap();
        r.state_mut()
            .objects
            .get_mut(&card)
            .unwrap()
            .counters
            .insert(CounterType::Time, 1);
        let action = legal_actions(r.state()).into_iter().find(|a| {
            matches!(a, GameAction::ActivateAbility { source_id, .. } if *source_id == card)
        }).expect("suspended Gargadon's sacrifice");
        r.act(action).unwrap();
        r.act(GameAction::SelectCards {
            cards: vec![mountain],
        })
        .unwrap();
        assert_eq!(r.state().objects[&mountain].zone, Zone::Graveyard);
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), card), 0);
        assert_eq!(r.state().stack.len(), 1);
        assert_eq!(r.state().players[0].mana_pool.total(), 0);
        pass_pair(&mut r);
        if let Some(plains) = plains {
            assert!(matches!(
                r.state().waiting_for,
                WaitingFor::OptionalEffectChoice { player: P0, .. }
            ));
            assert!(!r.state().objects[&plains].tapped);
            r.act(GameAction::DecideOptionalEffect { accept: true })
                .unwrap();
            assert_eq!(r.state().stack.back().unwrap().id, card);
            assert_eq!(r.state().objects[&card].zone, Zone::Stack);
            assert!(r.state().objects[&plains].tapped);
            assert_eq!(r.state().players[0].mana_pool.total(), 0);
        } else {
            assert!(
                matches!(r.state().waiting_for, WaitingFor::Priority { .. }),
                "unpayable cast was offered: {:?}",
                r.state().waiting_for
            );
            assert!(r.state().stack.is_empty());
            assert_eq!(r.state().objects[&card].zone, Zone::Exile);
            assert!(r.state().objects[&card].casting_permissions.is_empty());
        }
    }
}

#[test]
fn suspend_last_counter_requires_the_same_exiled_card() {
    let (pending, card) = gargadon_at_last_counter(|_| {});
    for reenter in [false, true] {
        let mut r = GameRunner::from_state(pending.state().clone());
        engine::game::zones::move_to_zone(r.state_mut(), card, Zone::Graveyard, &mut vec![]);
        if reenter {
            engine::game::zones::move_to_zone(r.state_mut(), card, Zone::Exile, &mut vec![]);
        }
        pass_pair(&mut r);
        assert!(
            matches!(r.state().waiting_for, WaitingFor::Priority { .. }),
            "a departed incarnation cannot offer its play: {:?}",
            r.state().waiting_for
        );
        assert!(r.state().stack.is_empty());
        assert!(r.state().objects[&card].casting_permissions.is_empty());
    }
    let mut r = pending;
    r.state_mut()
        .objects
        .get_mut(&card)
        .unwrap()
        .counters
        .insert(CounterType::Time, 1);
    pass_pair(&mut r);
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::OptionalEffectChoice { player: P0, .. }
    ));
    r.act(GameAction::DecideOptionalEffect { accept: true })
        .unwrap();
    assert_eq!(r.state().objects[&card].zone, Zone::Stack);
}

#[test]
fn suspend_last_counter_can_play_a_granted_dryad_arbor() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_basic_land(P0, ManaColor::Blue);
    let arbor = printed_card(&mut s, P0, "Dryad Arbor", Zone::Battlefield);
    let spell = printed_card(&mut s, P0, "Suspend", Zone::Hand);
    let mut r = s.build();
    r.cast(spell).target_object(arbor).resolve();
    assert_eq!(r.state().objects[&arbor].zone, Zone::Exile);
    assert_eq!(time_counters(r.state(), arbor), 2);
    one_counter_upkeep(&mut r, arbor);
    pass_pair(&mut r);
    assert_eq!(time_counters(r.state(), arbor), 0);
    assert_eq!(r.state().stack.len(), 1);
    for other_turn in [false, true] {
        let mut blocked = GameRunner::from_state(r.state().clone());
        if other_turn {
            blocked.state_mut().active_player = P1;
        } else {
            blocked.state_mut().lands_played_this_turn = 1;
        }
        pass_pair(&mut blocked);
        assert!(matches!(
            blocked.state().waiting_for,
            WaitingFor::Priority { .. }
        ));
        assert_eq!(blocked.state().objects[&arbor].zone, Zone::Exile);
        assert!(blocked.state().objects[&arbor]
            .casting_permissions
            .is_empty());
    }
    pass_pair(&mut r);
    assert!(
        matches!(
            r.state().waiting_for,
            WaitingFor::OptionalEffectChoice { player: P0, .. }
        ),
        "legal land play must be offered: {:?}",
        r.state().waiting_for
    );
    r.act(GameAction::DecideOptionalEffect { accept: true })
        .unwrap();
    assert_eq!(r.state().objects[&arbor].zone, Zone::Battlefield);
    assert!(r.state().stack.is_empty());
    assert_eq!(r.state().lands_played_this_turn, 1);
}

#[test]
fn suspend_last_counter_copy_plays_for_its_controller() {
    use engine::types::ability::AbilityTag;
    use engine::types::game_state::StackEntryKind;

    let mut copy_spell = None;
    let (mut r, card) = gargadon_at_last_counter(|s| {
        copy_spell = Some(printed_card(s, P1, "Return the Favor", Zone::Hand));
        for _ in 0..3 {
            s.add_basic_land(P1, ManaColor::Red);
        }
    });
    let original = r.state().stack.back().unwrap().id;
    r.act(GameAction::PassPriority).unwrap();
    r.cast(copy_spell.unwrap())
        .modes(&[0])
        .target_object(original)
        .commit();
    assert_eq!(r.state().stack.len(), 2);
    pass_pair(&mut r);
    assert_eq!(r.state().stack.len(), 2);
    for (entry, player) in r.state().stack.iter().zip([P0, P1]) {
        assert_eq!(entry.controller, player);
        let StackEntryKind::TriggeredAbility { ability, .. } = &entry.kind else {
            panic!("expected triggered ability: {:?}", entry.kind)
        };
        assert_eq!(ability.controller, player);
        assert_eq!(
            ability.context.ability_tag,
            Some(AbilityTag::SuspendLastCounter)
        );
        assert_eq!(ability.source_id, card);
    }
    pass_pair(&mut r);
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::OptionalEffectChoice { player: P1, .. }
    ));
    let mut declined = GameRunner::from_state(r.state().clone());
    declined
        .act(GameAction::DecideOptionalEffect { accept: false })
        .unwrap();
    pass_pair(&mut declined);
    assert!(matches!(
        declined.state().waiting_for,
        WaitingFor::OptionalEffectChoice { player: P0, .. }
    ));
    r.act(GameAction::DecideOptionalEffect { accept: true })
        .unwrap();
    assert_eq!(r.state().stack.back().unwrap().id, card);
    assert_eq!(r.state().stack.back().unwrap().controller, P1);
    assert_eq!(r.state().objects[&card].owner, P0);
    pass_pair(&mut r);
    assert_eq!(r.state().objects[&card].zone, Zone::Battlefield);
    pass_pair(&mut r);
    assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
}

#[test]
fn suspend_last_counter_rejects_a_prohibited_creature_spell() {
    let mut golem = None;
    let (mut r, card) = gargadon_at_last_counter(|s| {
        golem = Some(printed_card(s, P0, "Steel Golem", Zone::Hand));
    });
    // Install the printed prohibition after the legal hand action, before the trigger resolves.
    engine::game::zones::move_to_zone(
        r.state_mut(),
        golem.unwrap(),
        Zone::Battlefield,
        &mut vec![],
    );
    pass_pair(&mut r);
    assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
    assert_eq!(r.state().objects[&card].zone, Zone::Exile);
    assert!(r.state().objects[&card].casting_permissions.is_empty());
}

fn restore_suspend_save(saved: PersistedGameState, with_db: bool) -> GameRunner {
    use engine::types::game_state::PersistedRestoreFinalization;
    let state = if with_db {
        saved
            .prepare_for_restore(PersistedRestoreFinalization::DeferUntilRehydrated)
            .unwrap()
            .finalize_after_rehydration(|state| {
                engine::game::rehydrate_game_from_card_db_with_finalization(
                    state,
                    crate::support::shared_card_db().unwrap(),
                    engine::game::CardDbRehydrationFinalization::Defer,
                );
                Ok(())
            })
            .unwrap()
    } else {
        saved.into_game_state().unwrap()
    };
    GameRunner::from_state(state)
}

#[test]
fn suspend_review_historical_install_generates_the_correct_future_trigger_on_first_load() {
    use engine::types::ability::AbilityTag;
    use engine::types::game_state::StackEntryKind;
    let card = ObjectId(2);
    for with_db in [false, true] {
        let mut r = restore_suspend_save(
            serde_json::from_str(include_str!(
                "../fixtures/activation_cost/greater_gargadon_pre_special_action_manual.json"
            ))
            .unwrap(),
            with_db,
        );
        let mountain = r.state().battlefield[0];
        r.act(mana_source(&r, mountain, ManaType::Red)).unwrap();
        r.act(GameAction::PassPriority).unwrap();
        one_counter_upkeep(&mut r, card);
        pass_pair(&mut r);
        let StackEntryKind::TriggeredAbility { ability, .. } =
            &r.state().stack.back().unwrap().kind
        else {
            panic!("expected the last-counter ability")
        };
        assert_eq!(
            ability.context.ability_tag,
            Some(AbilityTag::SuspendLastCounter)
        );
        engine::game::zones::move_to_zone(r.state_mut(), card, Zone::Graveyard, &mut vec![]);
        pass_pair(&mut r);
        assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
    }
}

// Exported at the two suspension points by the historical writer; gzip -9 -n.
fn old_suspend_save(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_reader(flate2::read::GzDecoder::new(bytes)).unwrap()
}

#[test]
fn suspend_review_restores_the_pending_keyword_on_the_stack_or_at_the_choice() {
    use engine::types::ability::AbilityTag;
    use engine::types::game_state::StackEntryKind;
    let card = ObjectId(2);
    let trigger = old_suspend_save(include_bytes!(
        "../fixtures/activation_cost/greater_gargadon_pre_intervening_trigger.json.gz"
    ));
    let choice = old_suspend_save(include_bytes!(
        "../fixtures/activation_cost/greater_gargadon_pre_intervening_choice.json.gz"
    ));
    for with_db in [false, true] {
        let mut r = restore_suspend_save(serde_json::from_value(trigger.clone()).unwrap(), with_db);
        let StackEntryKind::TriggeredAbility { ability, .. } =
            &r.state().stack.back().unwrap().kind
        else {
            panic!("expected the restored last-counter ability")
        };
        assert_eq!(
            ability.context.ability_tag,
            Some(AbilityTag::SuspendLastCounter)
        );
        engine::game::zones::move_to_zone(r.state_mut(), card, Zone::Graveyard, &mut vec![]);
        pass_pair(&mut r);
        assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));

        let mut r = restore_suspend_save(serde_json::from_value(choice.clone()).unwrap(), with_db);
        assert!(matches!(
            r.state().waiting_for,
            WaitingFor::OptionalEffectChoice { player: P0, .. }
        ));
        assert_eq!(
            r.state()
                .resolution_stack
                .active_optional_effect()
                .unwrap()
                .ability
                .context
                .ability_tag,
            Some(AbilityTag::SuspendLastCounter)
        );
        r.act(GameAction::DecideOptionalEffect { accept: true })
            .unwrap();
        assert_eq!(r.state().objects[&card].zone, Zone::Stack);
    }
}

#[test]
fn suspend_review_restoring_a_departed_parked_choice_finishes_resolution() {
    let choice = old_suspend_save(include_bytes!(
        "../fixtures/activation_cost/greater_gargadon_pre_intervening_choice.json.gz"
    ));
    let card = ObjectId(2);
    for with_db in [false, true] {
        for reenter in [false, true] {
            // Preserve the historical parked frame and move its source through the zone pipeline.
            let PersistedGameState::Raw(mut old) =
                serde_json::from_value(choice["state"].clone()).unwrap()
            else {
                panic!("expected the raw state inside the historical envelope")
            };
            engine::game::zones::move_to_zone(&mut old, card, Zone::Graveyard, &mut vec![]);
            if reenter {
                engine::game::zones::move_to_zone(&mut old, card, Zone::Exile, &mut vec![]);
            }
            let r = restore_suspend_save(PersistedGameState::capture(*old), with_db);
            assert!(
                matches!(r.state().waiting_for, WaitingFor::Priority { player: P0 }),
                "db={with_db} reentry={reenter}: {:?}",
                r.state().waiting_for
            );
            assert!(
                r.state().resolution_stack.is_empty() && r.state().resolving_stack_entry.is_none()
            );
            assert!(
                r.state().stack.is_empty()
                    && r.state().objects[&card].casting_permissions.is_empty()
            );
            assert_eq!(
                r.state().objects[&card].zone,
                if reenter {
                    Zone::Exile
                } else {
                    Zone::Graveyard
                }
            );
        }
    }
}

#[test]
fn suspend_last_counter_requires_and_charges_a_choosable_spree_mode() {
    const DANCE: &str = "Spree (Choose one or more additional costs.)\n\
+ {1} — Search your library for a basic land card or a Desert card, put it onto the battlefield, then shuffle.\n\
+ {3} — Create an X/X green Elemental creature token, where X is the number of lands you control.";
    const TRINISPHERE: &str = "As long as this artifact is untapped, each spell that would cost less than three mana to cast costs three mana to cast. (Additional mana in the cost may be paid with any color of mana or colorless mana. For example, a spell that would cost {1}{B} to cast costs {2}{B} to cast instead.)";
    for (with_floor, mana, can_pay) in [
        (false, 0, false),
        (false, 1, true),
        (true, 2, false),
        (true, 3, true),
    ] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        if with_floor {
            s.add_artifact_from_oracle(P1, "Trinisphere", TRINISPHERE);
        }
        let jhoira = printed_card(&mut s, P0, "Jhoira of the Ghitu", Zone::Battlefield);
        let dance = s
            .add_spell_to_hand_from_oracle(P0, "Dance of the Tumbleweeds", false, DANCE)
            .with_mana_cost(ManaCost::Cost {
                generic: 1,
                shards: vec![ManaCostShard::Green],
            })
            .id();
        s.with_mana_pool(
            P0,
            vec![engine::types::mana::ManaUnit::new(ManaType::Red, jhoira, false, vec![]); 2],
        );
        let lands: Vec<_> = (0..mana)
            .map(|_| s.add_basic_land(P0, ManaColor::Green))
            .collect();
        let mut r = s.build();
        r.act(hand_action(jhoira, 0, CastPaymentMode::Auto))
            .unwrap();
        r.act(GameAction::SelectCards { cards: vec![dance] })
            .unwrap();
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), dance), 4);
        one_counter_upkeep(&mut r, dance);
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), dance), 0);
        assert_eq!(r.state().stack.len(), 1);
        assert_eq!(r.state().players[0].mana_pool.total(), 0);
        pass_pair(&mut r);
        if can_pay {
            assert!(
                matches!(
                    r.state().waiting_for,
                    WaitingFor::OptionalEffectChoice { player: P0, .. }
                ),
                "expected a cast offer with {mana} mana, Trinisphere present: {with_floor}"
            );
            r.act(GameAction::DecideOptionalEffect { accept: true })
                .unwrap();
            assert!(matches!(
                r.state().waiting_for,
                WaitingFor::ModeChoice { player: P0, .. }
            ));
            r.act(GameAction::SelectModes { indices: vec![0] }).unwrap();
            let cast = r.state().stack.back().unwrap();
            assert_eq!(cast.id, dance);
            assert!(matches!(
                cast.kind,
                StackEntryKind::Spell { actual_mana_spent, .. } if actual_mana_spent == mana
            ));
            assert!(lands.iter().all(|id| r.state().objects[id].tapped));
            assert_eq!(r.state().players[0].mana_pool.total(), 0);
        } else {
            assert!(
                matches!(r.state().waiting_for, WaitingFor::Priority { .. }),
                "{:?}",
                r.state().waiting_for
            );
            assert_eq!(r.state().objects[&dance].zone, Zone::Exile);
            assert!(r.state().objects[&dance].casting_permissions.is_empty());
            assert!(r.state().stack.is_empty());
            assert!(lands.iter().all(|id| !r.state().objects[id].tapped));
        }
    }
}

#[test]
fn suspend_last_counter_prices_a_spree_mode_with_its_creature_target() {
    use engine::types::ability::TargetRef;

    const RAMPAGE: &str = "Spree (Choose one or more additional costs.)\n\
+ {1} — Untap all creatures target player controls.\n\
+ {1} — Target creature gains double strike until end of turn.";
    for with_reduction in [false, true] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        let jhoira = printed_card(&mut s, P0, "Jhoira of the Ghitu", Zone::Battlefield);
        if with_reduction {
            s.add_creature_from_oracle(P0, "Killian, Ink Duelist", 2, 2, KILLIAN_ORACLE);
        }
        let rampage = s
            .add_spell_to_hand_from_oracle(P0, "Rustler Rampage", true, RAMPAGE)
            .with_mana_cost(ManaCost::Cost {
                generic: 0,
                shards: vec![ManaCostShard::White],
            })
            .id();
        s.with_mana_pool(
            P0,
            vec![engine::types::mana::ManaUnit::new(ManaType::Red, jhoira, false, vec![]); 2],
        );
        let mut r = s.build();
        r.act(hand_action(jhoira, 0, CastPaymentMode::Auto))
            .unwrap();
        r.act(GameAction::SelectCards {
            cards: vec![rampage],
        })
        .unwrap();
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), rampage), 4);
        one_counter_upkeep(&mut r, rampage);
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), rampage), 0);
        assert_eq!(r.state().stack.len(), 1);
        assert_eq!(r.state().players[0].mana_pool.total(), 0);
        pass_pair(&mut r);
        if with_reduction {
            assert!(
                matches!(
                    r.state().waiting_for,
                    WaitingFor::OptionalEffectChoice { player: P0, .. }
                ),
                "Killian makes the creature-targeting mode payable: {:?}",
                r.state().waiting_for
            );
            r.act(GameAction::DecideOptionalEffect { accept: true })
                .unwrap();
            r.act(GameAction::SelectModes { indices: vec![1] }).unwrap();
            assert!(matches!(
                r.state().waiting_for,
                WaitingFor::TargetSelection { player: P0, .. }
            ));
            r.act(GameAction::ChooseTarget {
                target: Some(TargetRef::Object(jhoira)),
            })
            .unwrap();
            let cast = r.state().stack.back().unwrap();
            assert_eq!(cast.id, rampage);
            assert!(matches!(
                cast.kind,
                StackEntryKind::Spell {
                    actual_mana_spent: 0,
                    ..
                }
            ));
            assert_eq!(r.state().objects[&rampage].zone, Zone::Stack);
        } else {
            assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
            assert_eq!(r.state().objects[&rampage].zone, Zone::Exile);
            assert!(r.state().objects[&rampage].casting_permissions.is_empty());
            assert!(r.state().stack.is_empty());
        }
    }
}

#[test]
fn suspend_review_wren_requires_a_payable_branch_and_charges_its_mana() {
    for count in [2, 3] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        let jhoira = printed_card(&mut s, P0, "Jhoira of the Ghitu", Zone::Battlefield);
        let wren = s.add_creature_to_hand_from_oracle(
            P0, "Wren's Run Vanquisher", 3, 3,
            "As an additional cost to cast this spell, reveal an Elf card from your hand or pay {3}.\nDeathtouch",
        ).with_mana_cost(ManaCost::Cost { generic: 1, shards: vec![ManaCostShard::Green] }).with_subtypes(vec!["Elf", "Warrior"]).id();
        let lands: Vec<_> = (0..count)
            .map(|_| s.add_basic_land(P0, ManaColor::Red))
            .collect();
        // Jhoira spends the floated mana. The lands price only the subsequent spell.
        s.with_mana_pool(
            P0,
            vec![engine::types::mana::ManaUnit::new(ManaType::Red, lands[0], false, vec![]); 2],
        );
        let mut r = s.build();
        r.act(hand_action(jhoira, 0, CastPaymentMode::Auto))
            .unwrap();
        r.act(GameAction::SelectCards { cards: vec![wren] })
            .unwrap();
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), wren), 4);
        one_counter_upkeep(&mut r, wren);
        assert!(lands.iter().all(|id| !r.state().objects[id].tapped));
        assert_eq!(r.state().players[0].mana_pool.total(), 0);
        pass_pair(&mut r);
        pass_pair(&mut r);
        if count == 2 {
            assert!(
                matches!(r.state().waiting_for, WaitingFor::Priority { .. }),
                "{:?}",
                r.state().waiting_for
            );
            assert_eq!(r.state().objects[&wren].zone, Zone::Exile);
            assert!(r.state().objects[&wren].casting_permissions.is_empty());
        } else {
            assert!(matches!(
                r.state().waiting_for,
                WaitingFor::OptionalEffectChoice { player: P0, .. }
            ));
            r.act(GameAction::DecideOptionalEffect { accept: true })
                .unwrap();
            assert_eq!(
                r.state().objects[&wren].zone,
                Zone::Stack,
                "{:?}",
                r.state().waiting_for
            );
            assert!(lands.iter().all(|id| r.state().objects[id].tapped));
        }
    }
}

#[test]
fn suspend_last_counter_plays_the_elected_modal_face_immediately() {
    use engine::types::card_type::CoreType;
    // Both real Pathway faces and the Mammoth's land back are in the standard fixture.
    for name in ["Branchloft Pathway", "Kazandu Mammoth"] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        s.add_basic_land(P0, ManaColor::Blue);
        s.add_enchantment_from_oracle(
            P0,
            "Living Plane",
            "All lands are 1/1 creatures that are still lands.",
        );
        let modal = printed_card(&mut s, P0, name, Zone::Battlefield);
        let spell = printed_card(&mut s, P0, "Suspend", Zone::Hand);
        let mut r = s.build();
        engine::game::layers::mark_layers_full(r.state_mut());
        engine::game::layers::flush_layers(r.state_mut());
        assert!(r.state().objects[&modal]
            .card_types
            .core_types
            .contains(&CoreType::Creature));
        let back_name = r.state().objects[&modal]
            .back_face
            .as_ref()
            .unwrap()
            .name
            .clone();
        r.cast(spell).target_object(modal).resolve();
        assert_eq!(r.state().objects[&modal].zone, Zone::Exile);
        one_counter_upkeep(&mut r, modal);
        pass_pair(&mut r);
        let mut no_land = GameRunner::from_state(r.state().clone());
        no_land.state_mut().lands_played_this_turn = 1;
        pass_pair(&mut no_land);
        if name == "Kazandu Mammoth" {
            assert!(matches!(
                no_land.state().waiting_for,
                WaitingFor::OptionalEffectChoice { .. }
            ));
            no_land
                .act(GameAction::DecideOptionalEffect { accept: true })
                .unwrap();
            assert_eq!(no_land.state().objects[&modal].zone, Zone::Stack);
        } else {
            assert!(matches!(
                no_land.state().waiting_for,
                WaitingFor::Priority { .. }
            ));
        }
        pass_pair(&mut r);
        r.act(GameAction::DecideOptionalEffect { accept: true })
            .unwrap();
        assert!(
            matches!(r.state().waiting_for, WaitingFor::ModalFaceChoice { .. }),
            "both faces must be elected now: {:?}",
            r.state().waiting_for
        );
        for back_face in [false, true] {
            let mut face = GameRunner::from_state(r.state().clone());
            face.act(GameAction::ChooseModalFace { back_face }).unwrap();
            let played_land = name == "Branchloft Pathway" || back_face;
            assert_eq!(
                face.state().objects[&modal].zone,
                if played_land {
                    Zone::Battlefield
                } else {
                    Zone::Stack
                }
            );
            assert_eq!(face.state().lands_played_this_turn, u8::from(played_land));
            assert_eq!(
                face.state().objects[&modal].name,
                if back_face { back_name.as_str() } else { name }
            );
        }
    }
}

#[test]
fn suspend_last_counter_two_paid_sacrifices_trigger_only_once() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let mountain = s.add_basic_land(P0, ManaColor::Red);
    let bear = s.add_creature(P0, "Grizzly Bears", 2, 2).id();
    let card = gargadon_in_hand(&mut s);
    let mut r = s.build();
    r.act(hand_action(
        card,
        suspend_ability_index(r.state(), card),
        CastPaymentMode::Auto,
    ))
    .unwrap();
    r.state_mut()
        .objects
        .get_mut(&card)
        .unwrap()
        .counters
        .insert(CounterType::Time, 1);
    for permanent in [mountain, bear] {
        let action = legal_actions(r.state())
            .into_iter()
            .find(|a| matches!(a,GameAction::ActivateAbility{source_id,..} if *source_id==card))
            .expect("Gargadon's published sacrifice");
        r.act(action).unwrap();
        r.act(GameAction::SelectCards {
            cards: vec![permanent],
        })
        .unwrap();
        assert_eq!(r.state().objects[&permanent].zone, Zone::Graveyard);
        assert_eq!(time_counters(r.state(), card), 1);
    }
    assert_eq!(r.state().stack.len(), 2);
    pass_pair(&mut r);
    assert_eq!(
        r.state().stack.len(),
        2,
        "new last-counter trigger above the older sacrifice"
    );
    assert_eq!(time_counters(r.state(), card), 0);
    pass_pair(&mut r);
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::OptionalEffectChoice { player: P0, .. }
    ));
    let mut accepted = GameRunner::from_state(r.state().clone());
    accepted
        .act(GameAction::DecideOptionalEffect { accept: true })
        .unwrap();
    assert_eq!(accepted.state().stack.len(), 2);
    assert_eq!(accepted.state().stack.back().unwrap().id, card);
    r.act(GameAction::DecideOptionalEffect { accept: false })
        .unwrap();
    assert_eq!(r.state().stack.len(), 1);
    pass_pair(&mut r);
    assert!(r.state().stack.is_empty());
    assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
    assert_eq!(r.state().objects[&card].zone, Zone::Exile);
    assert!(r.state().objects[&card].casting_permissions.is_empty());
}

#[test]
fn suspend_last_counter_additional_cost_and_target_are_paid_during_the_spell_cast() {
    use engine::types::ability::TargetRef;
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_basic_land(P0, ManaColor::Red);
    s.add_basic_land(P0, ManaColor::Blue);
    let jhoira = printed_card(&mut s, P0, "Jhoira of the Ghitu", Zone::Battlefield);
    let bone = printed_card(&mut s, P0, "Bone Splinters", Zone::Hand);
    let target = s.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let mut r = s.build();
    r.act(hand_action(jhoira, 0, CastPaymentMode::Auto))
        .unwrap();
    r.act(GameAction::SelectCards { cards: vec![bone] })
        .unwrap();
    pass_pair(&mut r);
    assert_eq!(time_counters(r.state(), bone), 4);
    one_counter_upkeep(&mut r, bone);
    pass_pair(&mut r);
    let mut unpaid = GameRunner::from_state(r.state().clone());
    engine::game::zones::move_to_zone(unpaid.state_mut(), jhoira, Zone::Graveyard, &mut vec![]);
    pass_pair(&mut unpaid);
    assert!(
        matches!(unpaid.state().waiting_for, WaitingFor::Priority { .. }),
        "no creature to sacrifice: {:?}",
        unpaid.state().waiting_for
    );
    assert_eq!(unpaid.state().objects[&bone].zone, Zone::Exile);
    pass_pair(&mut r);
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::OptionalEffectChoice { player: P0, .. }
    ));
    r.act(GameAction::DecideOptionalEffect { accept: true })
        .unwrap();
    let mut paid = false;
    let mut targeted = false;
    for _ in 0..3 {
        match r.state().waiting_for {
            WaitingFor::TargetSelection { .. } => {
                r.act(GameAction::ChooseTarget {
                    target: Some(TargetRef::Object(target)),
                })
                .unwrap();
                targeted = true;
            }
            WaitingFor::PayCost { .. } => {
                r.act(GameAction::SelectCards {
                    cards: vec![jhoira],
                })
                .unwrap();
                paid = true;
            }
            WaitingFor::Priority { .. } => break,
            ref other => panic!("unexpected Bone Splinters prompt {other:?}"),
        }
    }
    assert!(paid && targeted);
    assert_eq!(r.state().objects[&jhoira].zone, Zone::Graveyard);
    assert_eq!(r.state().objects[&target].zone, Zone::Battlefield);
    assert_eq!(r.state().objects[&bone].zone, Zone::Stack);
    pass_pair(&mut r);
    assert_eq!(r.state().objects[&target].zone, Zone::Graveyard);
}

#[test]
fn suspend_last_counter_prices_a_required_sacrifice_with_its_creature_target() {
    use engine::types::ability::TargetRef;

    const YASHARN: &str = "When Yasharn enters, search your library for a basic Forest card and a basic Plains card, reveal those cards, put them into your hand, then shuffle.\nPlayers can't pay life or sacrifice nonland permanents to cast spells or activate abilities.";
    for (is_aura, with_reduction, sacrifice_prohibited) in [
        (false, true, false),
        (false, false, false),
        (false, true, true),
        (true, true, false),
    ] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        let jhoira = printed_card(&mut s, P0, "Jhoira of the Ghitu", Zone::Battlefield);
        let spell = if is_aura {
            s.add_enchantment_to_hand_from_oracle(P0, "Grafted Identity", "")
                .with_subtypes(vec!["Aura"])
                .from_oracle_text_with_keywords(&["enchant"], GRAFTED_IDENTITY)
                .with_mana_cost(ManaCost::Cost {
                    generic: 2,
                    shards: vec![ManaCostShard::Blue, ManaCostShard::Blue],
                })
                .id()
        } else {
            printed_card(&mut s, P0, "Bone Splinters", Zone::Hand)
        };
        if with_reduction {
            s.add_creature_from_oracle(P0, "Killian, Ink Duelist", 2, 2, KILLIAN_ORACLE);
        }
        if sacrifice_prohibited {
            s.add_creature_from_oracle(P1, "Yasharn, Implacable Earth", 4, 4, YASHARN);
        }
        s.add_artifact_from_oracle(P1, "Sphere of Resistance", "Spells cost {1} more to cast.");
        let target = s.add_creature(P1, "Grizzly Bears", 2, 2).id();
        s.with_mana_pool(
            P0,
            vec![engine::types::mana::ManaUnit::new(ManaType::Red, jhoira, false, vec![]); 2],
        );

        let mut r = s.build();
        r.act(hand_action(jhoira, 0, CastPaymentMode::Auto))
            .unwrap();
        r.act(GameAction::SelectCards { cards: vec![spell] })
            .unwrap();
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), spell), 4);
        if is_aura {
            assert!(r.state().objects[&spell].keywords.iter().any(|keyword| {
                matches!(keyword, engine::types::keywords::Keyword::Enchant(_))
            }));
        }
        one_counter_upkeep(&mut r, spell);
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), spell), 0);
        assert_eq!(r.state().stack.len(), 1);
        assert_eq!(r.state().stack.back().unwrap().source_id, spell);
        assert_eq!(r.state().players[0].mana_pool.total(), 0);
        pass_pair(&mut r);

        if !with_reduction || sacrifice_prohibited {
            assert!(
                matches!(r.state().waiting_for, WaitingFor::Priority { .. }),
                "reduction={with_reduction} sacrifice prohibited={sacrifice_prohibited}: {:?}",
                r.state().waiting_for
            );
            assert!(r.state().stack.is_empty());
            assert_eq!(r.state().objects[&spell].zone, Zone::Exile);
            assert!(r.state().objects[&spell].casting_permissions.is_empty());
            assert_eq!(r.state().objects[&jhoira].zone, Zone::Battlefield);
            continue;
        }

        assert!(
            matches!(
                r.state().waiting_for,
                WaitingFor::OptionalEffectChoice { player: P0, .. }
            ),
            "Killian covers Sphere's tax and Jhoira can be sacrificed (Aura={is_aura}): {:?}",
            r.state().waiting_for
        );
        r.act(GameAction::DecideOptionalEffect { accept: true })
            .unwrap();
        assert!(matches!(
            r.state().waiting_for,
            WaitingFor::TargetSelection { player: P0, .. }
        ));
        r.act(GameAction::ChooseTarget {
            target: Some(TargetRef::Object(target)),
        })
        .unwrap();
        assert!(matches!(
            r.state().waiting_for,
            WaitingFor::PayCost { player: P0, .. }
        ));
        let paid = r
            .act(GameAction::SelectCards {
                cards: vec![jhoira],
            })
            .unwrap();
        let sacrifices: Vec<_> = paid
            .events
            .iter()
            .filter_map(|event| match event {
                GameEvent::PermanentSacrificed {
                    object_id,
                    player_id,
                } => Some((*object_id, *player_id)),
                _ => None,
            })
            .collect();
        assert_eq!(sacrifices, vec![(jhoira, P0)]);
        assert_eq!(r.state().objects[&jhoira].zone, Zone::Graveyard);
        assert_eq!(r.state().objects[&target].zone, Zone::Battlefield);
        assert_eq!(r.state().objects[&spell].zone, Zone::Stack);
        assert_eq!(r.state().stack.len(), 1);
        let cast = r.state().stack.back().unwrap();
        assert_eq!(cast.id, spell);
        assert_eq!(cast.ability().unwrap().targets, [TargetRef::Object(target)]);
        assert!(matches!(
            cast.kind,
            StackEntryKind::Spell {
                actual_mana_spent: 0,
                ..
            }
        ));
        pass_pair(&mut r);
        if is_aura {
            assert_eq!(r.state().objects[&spell].zone, Zone::Battlefield);
            assert_eq!(r.state().objects[&target].controller, P0);
        } else {
            assert_eq!(r.state().objects[&target].zone, Zone::Graveyard);
        }
    }
}

#[test]
fn suspend_last_counter_prices_optional_exile_before_offering_the_cast() {
    const GOREX: &str = "As an additional cost to cast this spell, you may exile any number of creature cards from your graveyard. This spell costs {2} less to cast for each card exiled this way.\nDeathtouch\nWhenever Gorex attacks or dies, choose a card at random exiled with Gorex and put that card into its owner's hand.";
    for with_creature in [false, true] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        let jhoira = printed_card(&mut s, P0, "Jhoira of the Ghitu", Zone::Battlefield);
        let gorex = s
            .add_creature_to_hand_from_oracle(P0, "Gorex, the Tombshell", 4, 4, GOREX)
            .with_subtypes(vec!["Zombie", "Turtle"])
            .with_mana_cost(ManaCost::Cost {
                generic: 6,
                shards: vec![ManaCostShard::Black; 2],
            })
            .id();
        s.add_artifact_from_oracle(P1, "Sphere of Resistance", "Spells cost {1} more to cast.");
        let creature =
            with_creature.then(|| s.add_creature_to_graveyard(P0, "Grizzly Bears", 2, 2).id());
        s.with_mana_pool(
            P0,
            vec![engine::types::mana::ManaUnit::new(ManaType::Red, jhoira, false, vec![]); 2],
        );

        let mut r = s.build();
        r.act(hand_action(jhoira, 0, CastPaymentMode::Auto))
            .unwrap();
        r.act(GameAction::SelectCards { cards: vec![gorex] })
            .unwrap();
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), gorex), 4);
        one_counter_upkeep(&mut r, gorex);
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), gorex), 0);
        assert_eq!(r.state().stack.len(), 1);
        assert_eq!(r.state().stack.back().unwrap().source_id, gorex);
        assert_eq!(r.state().players[0].mana_pool.total(), 0);
        pass_pair(&mut r);

        let Some(creature) = creature else {
            assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
            assert!(r.state().stack.is_empty());
            assert_eq!(r.state().objects[&gorex].zone, Zone::Exile);
            assert!(r.state().objects[&gorex].casting_permissions.is_empty());
            continue;
        };
        assert!(
            matches!(
                r.state().waiting_for,
                WaitingFor::OptionalEffectChoice { player: P0, .. }
            ),
            "Gorex can reduce the tax to zero: {:?}",
            r.state().waiting_for
        );
        r.act(GameAction::DecideOptionalEffect { accept: true })
            .unwrap();
        assert!(matches!(
            r.state().waiting_for,
            WaitingFor::OptionalCostChoice { player: P0, .. }
        ));
        r.act(GameAction::DecideOptionalCost { index: 0 }).unwrap();
        let WaitingFor::PayCost {
            player: P0,
            choices,
            ..
        } = &r.state().waiting_for
        else {
            panic!(
                "expected the optional exile selection: {:?}",
                r.state().waiting_for
            );
        };
        assert_eq!(choices, &[creature]);
        r.act(GameAction::SelectCards {
            cards: vec![creature],
        })
        .unwrap();
        assert_eq!(r.state().objects[&creature].zone, Zone::Exile);
        assert_eq!(r.state().objects[&gorex].zone, Zone::Stack);
        assert_eq!(r.state().stack.len(), 1);
        let cast = r.state().stack.back().unwrap();
        assert_eq!(cast.id, gorex);
        assert!(matches!(
            cast.kind,
            StackEntryKind::Spell {
                actual_mana_spent: 0,
                ..
            }
        ));
    }
}

#[test]
fn normal_cost_conduit_cast_applies_killian_before_paying_grafted_identity() {
    use engine::types::ability::TargetRef;
    use engine::types::actions::CastChoice;
    use engine::types::game_state::CastOfferKind;

    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let conduit = printed_card(&mut s, P0, "Conduit of Worlds", Zone::Battlefield);
    let aura = s
        .add_enchantment_to_hand_from_oracle(P0, "Grafted Identity", "")
        .with_subtypes(vec!["Aura"])
        .from_oracle_text_with_keywords(&["enchant"], GRAFTED_IDENTITY)
        .with_mana_cost(ManaCost::Cost {
            generic: 2,
            shards: vec![ManaCostShard::Blue, ManaCostShard::Blue],
        })
        .id();
    let killian = s
        .add_creature_from_oracle(P0, "Killian, Ink Duelist", 2, 2, KILLIAN_ORACLE)
        .id();
    let sacrifice = s.add_creature(P0, "Runeclaw Bear", 2, 2).id();
    let target = s.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let islands = [
        s.add_basic_land(P0, ManaColor::Blue),
        s.add_basic_land(P0, ManaColor::Blue),
    ];

    let mut r = s.build();
    engine::game::zones::move_to_zone(r.state_mut(), aura, Zone::Graveyard, &mut vec![]);
    r.act(GameAction::ActivateAbility {
        source_id: conduit,
        ability_index: 0,
        payment_mode: CastPaymentMode::Auto,
    })
    .unwrap();
    pass_pair(&mut r);
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::CastOffer {
            player: P0,
            kind: CastOfferKind::GraveyardPaidCast { hit_card, .. },
            ..
        } if hit_card == aura
    ));
    assert_eq!(r.state().players[0].mana_pool.total(), 0);
    r.act(GameAction::GraveyardPaidCastChoice {
        choice: CastChoice::Cast,
    })
    .unwrap();
    assert!(
        matches!(
            &r.state().waiting_for,
            WaitingFor::TargetSelection { player: P0, .. }
        ),
        "the paid cast must reach its creature target election, got {:?}",
        r.state().waiting_for
    );
    r.act(GameAction::ChooseTarget {
        target: Some(TargetRef::Object(target)),
    })
    .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::PayCost { player: P0, .. }
    ));
    r.act(GameAction::SelectCards {
        cards: vec![sacrifice],
    })
    .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ManaPayment { player: P0, .. }
    ));
    for island in islands {
        r.act(mana_source(&r, island, ManaType::Blue)).unwrap();
    }

    let paid = r.act(GameAction::PassPriority).unwrap();
    let sacrifices: Vec<_> = paid
        .events
        .iter()
        .filter_map(|event| match event {
            GameEvent::PermanentSacrificed {
                object_id,
                player_id,
            } => Some((*object_id, *player_id)),
            _ => None,
        })
        .collect();
    assert_eq!(sacrifices, [(sacrifice, P0)]);
    assert_eq!(r.state().objects[&aura].zone, Zone::Stack);
    assert_eq!(r.state().objects[&sacrifice].zone, Zone::Graveyard);
    assert_eq!(r.state().objects[&killian].zone, Zone::Battlefield);
    assert_eq!(r.state().players[0].mana_pool.total(), 0);
    assert_eq!(r.state().stack.len(), 1);
    let cast = r.state().stack.back().unwrap();
    assert_eq!(cast.id, aura);
    assert_eq!(cast.ability().unwrap().targets, [TargetRef::Object(target)]);
    assert!(matches!(
        cast.kind,
        StackEntryKind::Spell {
            actual_mana_spent: 2,
            ..
        }
    ));
    pass_pair(&mut r);
    assert_eq!(r.state().objects[&aura].zone, Zone::Battlefield);
    assert_eq!(r.state().objects[&target].controller, P0);
    assert!(r.state().stack.is_empty());
    assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
}

#[test]
fn suspend_last_counter_prices_the_only_legal_target_before_offering_the_cast() {
    use engine::types::ability::TargetRef;

    const RONA: &str = "Kicker {2}{B} (You may pay an additional {2}{B} as you cast this spell.)\nReturn target creature or planeswalker you don't control to its owner's hand. If this spell was kicked, put that permanent on the bottom of its owner's library instead.";
    const KOPALA: &str = "Spells your opponents cast that target a Merfolk you control cost {2} more to cast.\nAbilities your opponents activate that target a Merfolk you control cost {2} more to activate.";
    for mana in [0, 2] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        let jhoira = printed_card(&mut s, P0, "Jhoira of the Ghitu", Zone::Battlefield);
        let vortex = s
            .add_spell_to_hand_from_oracle(P0, "Rona's Vortex", true, RONA)
            .with_mana_cost(ManaCost::Cost {
                generic: 0,
                shards: vec![ManaCostShard::Blue],
            })
            .id();
        let kopala = s
            .add_creature_from_oracle(P1, "Kopala, Warden of Waves", 2, 2, KOPALA)
            .with_subtypes(vec!["Merfolk", "Wizard"])
            .id();
        s.with_mana_pool(
            P0,
            vec![engine::types::mana::ManaUnit::new(ManaType::Red, jhoira, false, vec![]); 2],
        );
        let lands: Vec<_> = (0..mana)
            .map(|_| s.add_basic_land(P0, ManaColor::Blue))
            .collect();

        let mut r = s.build();
        r.act(hand_action(jhoira, 0, CastPaymentMode::Auto))
            .unwrap();
        r.act(GameAction::SelectCards {
            cards: vec![vortex],
        })
        .unwrap();
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), vortex), 4);
        one_counter_upkeep(&mut r, vortex);
        pass_pair(&mut r);
        assert_eq!(time_counters(r.state(), vortex), 0);
        assert_eq!(r.state().stack.len(), 1);
        assert_eq!(r.state().stack.back().unwrap().source_id, vortex);
        assert_eq!(r.state().players[0].mana_pool.total(), 0);
        pass_pair(&mut r);

        if mana == 0 {
            assert!(
                matches!(r.state().waiting_for, WaitingFor::Priority { .. }),
                "the only target requires two mana: {:?}",
                r.state().waiting_for
            );
            assert!(r.state().stack.is_empty());
            assert_eq!(r.state().objects[&vortex].zone, Zone::Exile);
            assert!(r.state().objects[&vortex].casting_permissions.is_empty());
            continue;
        }
        assert!(matches!(
            r.state().waiting_for,
            WaitingFor::OptionalEffectChoice { player: P0, .. }
        ));
        r.act(GameAction::DecideOptionalEffect { accept: true })
            .unwrap();
        // The engine elects the only legal target as part of the cast.
        assert_eq!(r.state().objects[&vortex].zone, Zone::Stack);
        assert_eq!(r.state().stack.len(), 1);
        let cast = r.state().stack.back().unwrap();
        assert_eq!(cast.id, vortex);
        assert_eq!(cast.ability().unwrap().targets, [TargetRef::Object(kopala)]);
        assert!(matches!(
            cast.kind,
            StackEntryKind::Spell {
                actual_mana_spent: 2,
                ..
            }
        ));
        assert!(lands.iter().all(|id| r.state().objects[id].tapped));
        pass_pair(&mut r);
        assert_eq!(r.state().objects[&kopala].zone, Zone::Hand);
    }
}

struct CrowdedSuspend {
    spell: ObjectId,
    jhoira: ObjectId,
    killian: ObjectId,
    discard: ObjectId,
    taxed: Vec<ObjectId>,
    untaxed: Option<[ObjectId; 2]>,
}

fn crowded_suspend_at_last_trigger(
    all_taxed: bool,
    sacrifice_prohibited: bool,
) -> (GameRunner, CrowdedSuspend) {
    const RUTHLESS_DISPOSAL: &str = "As an additional cost to cast this spell, discard a card and sacrifice a creature.\nTwo target creatures each get -13/-13 until end of turn.";
    const KOPALA: &str = "Spells your opponents cast that target a Merfolk you control cost {2} more to cast.\nAbilities your opponents activate that target a Merfolk you control cost {2} more to activate.";
    const STEELY: &str = "As Steely Resolve enters, choose a creature type.\nCreatures of the chosen type have shroud.";
    const YASHARN: &str = "When Yasharn enters, search your library for a basic Forest card and a basic Plains card, reveal those cards, put them into your hand, then shuffle.\nPlayers can't pay life or sacrifice nonland permanents to cast spells or activate abilities.";
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    let kopala = s
        .add_creature_from_oracle(P1, "Kopala, Warden of Waves", 2, 2, KOPALA)
        .with_subtypes(vec!["Merfolk", "Wizard"])
        .id();
    // Tokens available from repeated casts of Summon the School. Insert them
    // before any untaxed creature so the first 65 × 64 ordered pairs are taxed.
    let mut taxed = vec![kopala];
    for _ in 0..64 {
        taxed.push(
            s.add_creature(P1, "Merfolk Wizard", 1, 1)
                .with_subtypes(vec!["Merfolk", "Wizard"])
                .id(),
        );
    }
    let untaxed = (!all_taxed).then(|| {
        [
            s.add_creature(P1, "Grizzly Bears", 2, 2).id(),
            s.add_creature(P1, "Hill Giant", 3, 3).id(),
        ]
    });
    let killian = s
        .add_creature_from_oracle(P0, "Killian, Ink Duelist", 2, 2, KILLIAN_ORACLE)
        .with_subtypes(vec!["Human", "Warlock"])
        .id();
    let steely = all_taxed.then(|| {
        s.add_enchantment_from_oracle(P1, "Steely Resolve", STEELY)
            .id()
    });
    if sacrifice_prohibited {
        s.add_creature_from_oracle(P1, "Yasharn, Implacable Earth", 4, 4, YASHARN);
    }
    let jhoira = printed_card(&mut s, P0, "Jhoira of the Ghitu", Zone::Battlefield);
    s.add_artifact_from_oracle(P1, "Sphere of Resistance", "Spells cost {1} more to cast.");
    let spell = s
        .add_spell_to_hand_from_oracle(P0, "Ruthless Disposal", false, RUTHLESS_DISPOSAL)
        .with_mana_cost(ManaCost::Cost {
            generic: 4,
            shards: vec![ManaCostShard::Black],
        })
        .id();
    let discard = s.add_card_to_hand(P0, "Plains");
    s.with_mana_pool(
        P0,
        vec![engine::types::mana::ManaUnit::new(ManaType::Red, jhoira, false, vec![]); 2],
    );

    let mut r = s.build();
    if let Some(steely) = steely {
        let state = r.state_mut();
        state
            .objects
            .get_mut(&steely)
            .unwrap()
            .chosen_attributes
            .push(engine::types::ability::ChosenAttribute::CreatureType(
                "Human".into(),
            ));
        engine::game::layers::mark_layers_full(state);
        engine::game::layers::flush_layers(state);
        for id in [jhoira, killian] {
            assert_eq!(r.state().objects[&id].zone, Zone::Battlefield);
            assert!(r.state().objects[&id].has_keyword(&engine::types::keywords::Keyword::Shroud));
        }
    }
    assert!(r.state().battlefield.iter().take(taxed.len()).eq(&taxed));
    assert!(taxed.len() * (taxed.len() - 1) > 4096);
    r.act(hand_action(jhoira, 0, CastPaymentMode::Auto))
        .unwrap();
    r.act(GameAction::SelectCards { cards: vec![spell] })
        .unwrap();
    pass_pair(&mut r);
    assert_eq!(time_counters(r.state(), spell), 4);
    one_counter_upkeep(&mut r, spell);
    pass_pair(&mut r);
    assert_eq!(time_counters(r.state(), spell), 0);
    assert_eq!(r.state().stack.len(), 1);
    assert_eq!(r.state().stack.back().unwrap().source_id, spell);
    assert_eq!(r.state().players[0].mana_pool.total(), 0);

    (
        r,
        CrowdedSuspend {
            spell,
            jhoira,
            killian,
            discard,
            taxed,
            untaxed,
        },
    )
}

fn assert_crowded_suspend_no_offer(mut r: GameRunner, board: &CrowdedSuspend) {
    assert_eq!(r.state().stack.back().unwrap().source_id, board.spell);
    pass_pair(&mut r);
    assert!(
        matches!(r.state().waiting_for, WaitingFor::Priority { .. }),
        "unpayable crowded cast offered: {:?}",
        r.state().waiting_for
    );
    assert!(r.state().stack.is_empty());
    assert_eq!(r.state().objects[&board.spell].zone, Zone::Exile);
    assert!(r.state().objects[&board.spell]
        .casting_permissions
        .is_empty());
    assert_eq!(r.state().objects[&board.jhoira].zone, Zone::Battlefield);
}

fn choose_crowded_suspend_targets(
    r: &mut GameRunner,
    board: &CrowdedSuspend,
    targets: [ObjectId; 2],
) -> Result<engine::types::game_state::ActionResult, engine::game::engine::EngineError> {
    use engine::types::ability::TargetRef;

    assert!(
        matches!(
            r.state().waiting_for,
            WaitingFor::OptionalEffectChoice { player: P0, .. }
        ),
        "expected a payable target pair: {:?}",
        r.state().waiting_for
    );
    r.act(GameAction::DecideOptionalEffect { accept: true })
        .unwrap();
    if board.untaxed.is_none() {
        let WaitingFor::TargetSelection { selection, .. } = &r.state().waiting_for else {
            panic!(
                "expected the Merfolk target prompt: {:?}",
                r.state().waiting_for
            );
        };
        assert_eq!(selection.current_legal_targets.len(), board.taxed.len());
        assert!(board.taxed.iter().all(|id| selection
            .current_legal_targets
            .contains(&TargetRef::Object(*id))));
    }
    let choose = |r: &mut GameRunner, id| {
        let target = TargetRef::Object(id);
        let WaitingFor::TargetSelection {
            player: P0,
            selection,
            ..
        } = &r.state().waiting_for
        else {
            panic!("expected target selection: {:?}", r.state().waiting_for);
        };
        assert!(selection.current_legal_targets.contains(&target));
        r.act(GameAction::ChooseTarget {
            target: Some(target),
        })
    };
    choose(r, targets[0]).unwrap();
    choose(r, targets[1])
}

fn pay_crowded_suspend(r: &mut GameRunner, board: &CrowdedSuspend, ids: [ObjectId; 2], mana: u32) {
    use engine::types::ability::TargetRef;
    use engine::types::game_state::PayCostKind;

    choose_crowded_suspend_targets(r, board, ids).unwrap();
    let targets = ids.map(TargetRef::Object);
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::PayCost {
            player: P0,
            kind: PayCostKind::Discard,
            ..
        }
    ));
    let discarded = r
        .act(GameAction::SelectCards {
            cards: vec![board.discard],
        })
        .unwrap();
    assert!(discarded.events.iter().any(|event| matches!(
        event, GameEvent::Discarded { player_id: P0, object_id, .. } if *object_id == board.discard
    )));
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::PayCost {
            player: P0,
            kind: PayCostKind::Sacrifice,
            ..
        }
    ));
    let sacrificed = r
        .act(GameAction::SelectCards {
            cards: vec![board.jhoira],
        })
        .unwrap();
    assert!(sacrificed.events.iter().any(|event| matches!(
        event, GameEvent::PermanentSacrificed { player_id: P0, object_id } if *object_id == board.jhoira
    )));
    assert_eq!(r.state().objects[&board.discard].zone, Zone::Graveyard);
    assert_eq!(r.state().objects[&board.jhoira].zone, Zone::Graveyard);
    assert_eq!(r.state().objects[&board.spell].zone, Zone::Stack);
    assert_eq!(r.state().stack.len(), 1);
    let cast = r.state().stack.back().unwrap();
    assert_eq!(cast.id, board.spell);
    assert_eq!(cast.ability().unwrap().targets, targets);
    assert!(matches!(
        cast.kind,
        StackEntryKind::Spell {
            actual_mana_spent,
            ..
        } if actual_mana_spent == mana
    ));
    assert_eq!(r.state().players[0].mana_pool.total(), 0);
}

#[test]
fn suspend_last_counter_required_cost_with_payable_targets_beyond_pricing_budget() {
    let (mut r, board) = crowded_suspend_at_last_trigger(false, false);
    for absent in [board.discard, board.killian] {
        let mut blocked = GameRunner::from_state(r.state().clone());
        engine::game::zones::move_to_zone(
            blocked.state_mut(),
            absent,
            Zone::Graveyard,
            &mut vec![],
        );
        assert_crowded_suspend_no_offer(blocked, &board);
    }
    let (blocked, sacrifice_board) = crowded_suspend_at_last_trigger(false, true);
    assert_crowded_suspend_no_offer(blocked, &sacrifice_board);
    pass_pair(&mut r);
    pay_crowded_suspend(&mut r, &board, board.untaxed.unwrap(), 0);
}

#[test]
fn suspend_last_counter_required_cost_preserves_unavoidable_target_tax_beyond_budget() {
    let (mut r, board) = crowded_suspend_at_last_trigger(true, false);
    assert_eq!(r.state().objects[&board.discard].zone, Zone::Hand);
    assert_crowded_suspend_no_offer(GameRunner::from_state(r.state().clone()), &board);

    r.state_mut().players[0]
        .mana_pool
        .add(engine::types::mana::ManaUnit::new(
            ManaType::Red,
            board.jhoira,
            false,
            vec![],
        ));
    pass_pair(&mut r);
    pay_crowded_suspend(&mut r, &board, [board.taxed[0], board.taxed[1]], 1);
}

#[test]
fn suspend_last_counter_required_cost_rejects_exact_unpayable_election_after_offer() {
    let (mut r, board) = crowded_suspend_at_last_trigger(false, false);
    pass_pair(&mut r);
    let second_target =
        choose_crowded_suspend_targets(&mut r, &board, [board.taxed[0], board.taxed[1]]);
    let result = second_target.unwrap();
    assert!(
        matches!(result.waiting_for, WaitingFor::Priority { .. }),
        "{result:?}"
    );
    assert!(!result.events.iter().any(|event| matches!(
        event,
        GameEvent::Discarded { .. } | GameEvent::PermanentSacrificed { .. }
    )));
    assert!(r.state().stack.is_empty());
    assert!(r.state().pending_cast.is_none());
    assert_eq!(r.state().objects[&board.spell].zone, Zone::Exile);
    assert!(r.state().objects[&board.spell]
        .casting_permissions
        .is_empty());
    assert_eq!(r.state().objects[&board.discard].zone, Zone::Hand);
    assert_eq!(r.state().objects[&board.jhoira].zone, Zone::Battlefield);
}

#[test]
fn suspend_upkeep_rechecks_exile_incarnation_and_effective_suspend() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_basic_land(P0, ManaColor::Red);
    let card = gargadon_in_hand(&mut s);
    let mut r = s.build();
    r.act(hand_action(
        card,
        suspend_ability_index(r.state(), card),
        CastPaymentMode::Auto,
    ))
    .unwrap();
    one_counter_upkeep(&mut r, card);
    for moved in [false, true] {
        let mut stale = GameRunner::from_state(r.state().clone());
        if moved {
            for zone in [Zone::Graveyard, Zone::Exile] {
                engine::game::zones::move_to_zone(stale.state_mut(), card, zone, &mut vec![]);
            }
            stale
                .state_mut()
                .objects
                .get_mut(&card)
                .unwrap()
                .counters
                .insert(CounterType::Time, 1);
        } else {
            // Isolate the effective-keyword axis while leaving the captured upkeep ability in flight.
            let obj = stale.state_mut().objects.get_mut(&card).unwrap();
            obj.keywords
                .retain(|kw| !matches!(kw, engine::types::keywords::Keyword::Suspend { .. }));
            obj.base_keywords
                .retain(|kw| !matches!(kw, engine::types::keywords::Keyword::Suspend { .. }));
        }
        pass_pair(&mut stale);
        assert_eq!(time_counters(stale.state(), card), 1);
        assert!(stale.state().stack.is_empty());
    }
    pass_pair(&mut r);
    assert_eq!(time_counters(r.state(), card), 0);
    assert_eq!(r.state().stack.len(), 1);
}

#[test]
fn stifling_each_suspend_trigger_preserves_the_corresponding_counter_total() {
    for counter_upkeep in [true, false] {
        let mut s = GameScenario::new();
        s.at_phase(Phase::PreCombatMain);
        s.add_basic_land(P0, ManaColor::Red);
        s.add_basic_land(P0, ManaColor::Blue);
        let card = gargadon_in_hand(&mut s);
        let stifle = printed_card(&mut s, P0, "Stifle", Zone::Hand);
        let mut r = s.build();
        r.act(hand_action(
            card,
            suspend_ability_index(r.state(), card),
            CastPaymentMode::Auto,
        ))
        .unwrap();
        one_counter_upkeep(&mut r, card);
        if !counter_upkeep {
            pass_pair(&mut r);
        }
        let trigger = r.state().stack.back().unwrap().id;
        r.cast(stifle).target_object(trigger).resolve();
        assert!(r.state().stack.is_empty());
        assert_eq!(r.state().objects[&card].zone, Zone::Exile);
        assert_eq!(time_counters(r.state(), card), u32::from(counter_upkeep));
        assert!(r.state().objects[&card].casting_permissions.is_empty());
        if counter_upkeep {
            one_counter_upkeep(&mut r, card);
            pass_pair(&mut r);
            assert_eq!(time_counters(r.state(), card), 0);
        }
    }
}

#[test]
fn rift_bolt_last_counter_cast_elects_and_damages_its_target() {
    use engine::types::ability::TargetRef;
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_basic_land(P0, ManaColor::Red);
    let bolt=s.add_spell_to_hand_from_oracle(P0,"Rift Bolt",false,"Rift Bolt deals 3 damage to any target.\nSuspend 1—{R} (Rather than cast this card from your hand, you may pay {R} and exile it with a time counter on it. At the beginning of your upkeep, remove a time counter. When the last is removed, you may cast it without paying its mana cost.)").with_mana_cost(ManaCost::Cost{generic:2,shards:vec![ManaCostShard::Red]}).id();
    let mut r = s.build();
    r.act(hand_action(
        bolt,
        suspend_ability_index(r.state(), bolt),
        CastPaymentMode::Auto,
    ))
    .unwrap();
    one_counter_upkeep(&mut r, bolt);
    pass_pair(&mut r);
    pass_pair(&mut r);
    r.act(GameAction::DecideOptionalEffect { accept: true })
        .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::TargetSelection { .. }
    ));
    r.act(GameAction::ChooseTarget {
        target: Some(TargetRef::Player(P1)),
    })
    .unwrap();
    assert_eq!(r.state().objects[&bolt].zone, Zone::Stack);
    let before = r.state().players[1].life;
    pass_pair(&mut r);
    assert_eq!(r.state().players[1].life, before - 3);
}

#[test]
fn riftmarked_knight_keeps_its_independent_printed_last_counter_trigger() {
    use engine::types::ability::AbilityTag;
    use engine::types::game_state::StackEntryKind;
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    for _ in 0..3 {
        s.add_basic_land(P0, ManaColor::White);
    }
    let knight=s.add_creature_to_hand_from_oracle(P0,"Riftmarked Knight",2,2,"Protection from black; flanking (Whenever a creature without flanking blocks this creature, the blocking creature gets -1/-1 until end of turn.)\nSuspend 3—{1}{W}{W}\nWhen the last time counter is removed from this card while it's exiled, create a 2/2 black Knight creature token with flanking, protection from white, and haste.").id();
    let mut r = s.build();
    r.act(hand_action(
        knight,
        suspend_ability_index(r.state(), knight),
        CastPaymentMode::Auto,
    ))
    .unwrap();
    one_counter_upkeep(&mut r, knight);
    pass_pair(&mut r);
    if matches!(r.state().waiting_for, WaitingFor::OrderTriggers { .. }) {
        r.act(GameAction::OrderTriggers { order: vec![0, 1] })
            .unwrap();
    }
    assert_eq!(r.state().stack.len(), 2);
    let tags: Vec<_> = r
        .state()
        .stack
        .iter()
        .map(|entry| {
            let StackEntryKind::TriggeredAbility { ability, .. } = &entry.kind else {
                panic!("expected trigger")
            };
            assert_eq!(ability.source_id, knight);
            ability.context.ability_tag
        })
        .collect();
    assert!(tags.contains(&Some(AbilityTag::SuspendLastCounter)) && tags.contains(&None));
    for _ in 0..2 {
        pass_pair(&mut r);
        if matches!(
            r.state().waiting_for,
            WaitingFor::OptionalEffectChoice { .. }
        ) {
            r.act(GameAction::DecideOptionalEffect { accept: false })
                .unwrap();
        }
    }
    assert_eq!(r.state().objects[&knight].zone, Zone::Exile);
    assert!(r
        .state()
        .battlefield
        .iter()
        .any(|id| r.state().objects[id].is_token && r.state().objects[id].name == "Knight"));
}

#[test]
fn suspend_upkeep_uses_each_cards_owner_after_a_grant_to_a_stolen_creature() {
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    for color in [ManaColor::Red, ManaColor::Red, ManaColor::Blue] {
        s.add_basic_land(P0, color);
    }
    let first = gargadon_in_hand(&mut s);
    let second = gargadon_in_hand(&mut s);
    let foreign = s
        .add_creature(P1, "Grizzly Bears", 2, 2)
        .controlled_by(P0)
        .id();
    let suspend = printed_card(&mut s, P0, "Suspend", Zone::Hand);
    let mut r = s.build();
    assert_eq!(r.state().objects[&foreign].controller, P0);
    for card in [first, second] {
        r.act(hand_action(
            card,
            suspend_ability_index(r.state(), card),
            CastPaymentMode::Auto,
        ))
        .unwrap();
    }
    r.state_mut()
        .objects
        .get_mut(&second)
        .unwrap()
        .counters
        .insert(CounterType::Time, 4);
    r.cast(suspend).target_object(foreign).resolve();
    assert_eq!(r.state().objects[&foreign].zone, Zone::Exile);
    assert_eq!(time_counters(r.state(), foreign), 2);
    for owner in [P0, P1] {
        let mut upkeep = GameRunner::from_state(r.state().clone());
        upkeep.state_mut().turn_number = 2;
        upkeep.state_mut().phase = Phase::Untap;
        upkeep.state_mut().active_player = owner;
        upkeep.state_mut().priority_player = owner;
        upkeep.state_mut().waiting_for = WaitingFor::Priority { player: owner };
        pass_pair(&mut upkeep);
        if matches!(upkeep.state().waiting_for, WaitingFor::OrderTriggers { .. }) {
            upkeep
                .act(GameAction::OrderTriggers { order: vec![0, 1] })
                .unwrap();
        }
        assert_eq!(upkeep.state().stack.len(), if owner == P0 { 2 } else { 1 });
        upkeep.advance_until_stack_empty();
        assert_eq!(
            time_counters(upkeep.state(), first),
            if owner == P0 { 9 } else { 10 }
        );
        assert_eq!(
            time_counters(upkeep.state(), second),
            if owner == P0 { 3 } else { 4 }
        );
        assert_eq!(
            time_counters(upkeep.state(), foreign),
            if owner == P0 { 2 } else { 1 }
        );
    }
}
