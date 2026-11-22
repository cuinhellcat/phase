//! Mold Folk — "Mold Harvest — {1}, Sacrifice another creature or an artifact:
//! Put a +1/+1 counter on this creature."
//!
//! CR 205.2a (a cost filter may be a two-leg card-TYPE union) + CR 601.2h (the
//! activation cost is paid with permanents matching that filter) + CR 701.21a
//! (sacrifice).
//!
//! The right conjunct of the union leads with an indefinite article, and the
//! shared type-phrase grammar deliberately leaves that tail as remainder — the
//! same surface is an elided-verb clause elsewhere ("you control a land creature
//! or a land entered the battlefield under your control this turn", Earth Rumble Wrestlers). The
//! sacrifice-cost parser now opts into the union reading via
//! `fold_article_led_type_union`, so the cost's filter is
//! `Or[Creature+Another, Artifact]` rather than `Creature+Another` alone.
//!
//! Note the ASYMMETRY, which is the rules-load-bearing part: "another" scopes only
//! the LEFT conjunct. The right one carries its own determiner ("an artifact"), so
//! it is not "another artifact" and the source may pay with ITSELF once it is an
//! artifact. Official rulings — Elite Headhunter (2019-10-04): "If Elite Headhunter
//! somehow becomes an artifact, you can sacrifice it to pay the cost of its
//! activated ability"; Gut, True Soul Zealot (2022-06-10): "If Gut somehow becomes
//! an artifact, you may sacrifice it to its own ability." The third test below
//! drives exactly that.
//!
//! Before the fix the artifact leg was dropped, so a controller holding only an
//! artifact could not pay a cost the card plainly allows. This drives the real
//! activation pipeline (`GameAction::ActivateAbility` → cost payment → stack →
//! resolution) rather than asserting a parsed filter shape.

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::Effect;
use engine::types::actions::GameAction;
use engine::types::card_type::CoreType;
use engine::types::counter::CounterType;
use engine::types::game_state::{CastPaymentMode, PayCostKind, StackEntryKind, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const MOLD_FOLK: &str = "Lifelink\n\
     Mold Harvest — {1}, Sacrifice another creature or an artifact: Put a +1/+1 counter on this creature.";

/// The artifact leg is REACHABLE: with no other creature on the battlefield, the
/// only legal sacrifice is the artifact, so the ability can be activated only if
/// the union kept its second leg.
///
/// Revert-failing: with the cost filter collapsed to `Creature+Another` the cost
/// is unpayable here, the ability never resolves, and the source gains no
/// counter while the artifact stays on the battlefield.
#[test]
fn mold_folk_pays_its_cost_by_sacrificing_an_artifact() {
    let mut scenario = GameScenario::new_n_player(2, 42);
    scenario.at_phase(Phase::PreCombatMain);
    let source = scenario
        .add_creature_from_oracle(P0, "Mold Folk", 1, 1, MOLD_FOLK)
        .id();
    let artifact = scenario.add_artifact_from_oracle(P0, "Bone Shard", "").id();
    // CR 602.1a: {1} for the activation cost (everything before the colon).
    scenario.with_mana_pool(
        P0,
        vec![ManaUnit::new(
            ManaType::Colorless,
            ObjectId(0),
            false,
            vec![],
        )],
    );
    let mut runner = scenario.build();

    // CR 601.2h: the artifact is submitted as the cost payment. The engine
    // accepts it only if the cost's filter actually admits an artifact.
    let outcome = runner.activate(source, 0).pay_with(&[artifact]).resolve();
    let state = outcome.state();

    assert_eq!(
        state.objects[&source]
            .counters
            .get(&CounterType::Plus1Plus1)
            .copied()
            .unwrap_or(0),
        1,
        "the ability must resolve, which it can only do if the artifact was a legal sacrifice"
    );
    assert_eq!(
        state.objects[&artifact].zone,
        Zone::Graveyard,
        "the artifact leg of the cost's type union must be sacrificeable"
    );
}

/// PAIRED CONTROL, the leg that already worked: with a creature available and no
/// artifact, the same ability still pays by sacrificing the creature. Proves the
/// union widened the filter rather than replacing one leg with the other.
#[test]
fn mold_folk_still_pays_its_cost_by_sacrificing_a_creature() {
    let mut scenario = GameScenario::new_n_player(2, 42);
    scenario.at_phase(Phase::PreCombatMain);
    let source = scenario
        .add_creature_from_oracle(P0, "Mold Folk", 1, 1, MOLD_FOLK)
        .id();
    let fodder = scenario.add_creature(P0, "Fodder", 1, 1).id();
    scenario.with_mana_pool(
        P0,
        vec![ManaUnit::new(
            ManaType::Colorless,
            ObjectId(0),
            false,
            vec![],
        )],
    );
    let mut runner = scenario.build();

    let outcome = runner.activate(source, 0).pay_with(&[fodder]).resolve();
    let state = outcome.state();

    assert_eq!(
        state.objects[&source]
            .counters
            .get(&CounterType::Plus1Plus1)
            .copied()
            .unwrap_or(0),
        1,
        "the creature leg must keep working"
    );
    assert_eq!(
        state.objects[&fodder].zone,
        Zone::Graveyard,
        "the creature was the sacrifice"
    );
    assert_eq!(
        state.objects[&source].zone,
        Zone::Battlefield,
        "\"another\" excludes the source itself"
    );
}

/// CR 205.2a + the Elite Headhunter / Gut rulings: "another" scopes only the LEFT
/// conjunct, so once the source itself is an artifact it satisfies the RIGHT leg
/// and may be sacrificed to its own ability.
///
/// This is the discriminating case for the `Another` scoping, and it is reachable
/// in real games — Liquimetal Coating and Mycosynth Lattice both make a creature
/// an artifact "in addition to its other types". (Not Karn, Silver Golem: that
/// one targets a NONcreature artifact and makes it a creature, the opposite
/// direction.)
///
/// GUARD-FAILING: distribute `FilterProp::Another` onto the article-led right leg
/// (i.e. fold the union *before* applying `another`, as an earlier revision did)
/// and the source matches neither leg — `matches_filter_prop`'s `Another` arm
/// resolves to `!source_is_current_object(..)` on this path — so
/// `find_eligible_sacrifice_targets` returns nothing, the cost is unpayable with
/// nothing else on the battlefield, and the activation is refused.
#[test]
fn mold_folk_may_sacrifice_itself_once_it_is_an_artifact() {
    let mut scenario = GameScenario::new_n_player(2, 42);
    scenario.at_phase(Phase::PreCombatMain);
    // Set as a fixture rather than parsed from a type-granting card: the subject
    // here is the COST FILTER, and routing through a continuous effect would put
    // the layer system in the path of a test that is not about it.
    // `as_artifact` REPLACES the creature type; `as_creature` puts it back
    // idempotently and re-syncs the base types, so the fixture is an artifact
    // CREATURE — the "in addition to its other types" state Liquimetal Coating and
    // Mycosynth Lattice produce, and the shape Street Urchin's own 2022-06-10
    // ruling describes ("If your commander is an artifact creature, you may
    // sacrifice it to pay the cost of this ability"). A pure artifact would
    // discriminate the same way but leave the both-types arm unexercised.
    let source = {
        let mut b = scenario.add_creature_from_oracle(P0, "Mold Folk", 1, 1, MOLD_FOLK);
        b.as_artifact();
        b.as_creature();
        b.id()
    };
    scenario.with_mana_pool(
        P0,
        vec![ManaUnit::new(
            ManaType::Colorless,
            ObjectId(0),
            false,
            vec![],
        )],
    );
    let mut runner = scenario.build();

    let core = &runner.state().objects[&source].card_types.core_types;
    assert!(
        core.contains(&CoreType::Artifact) && core.contains(&CoreType::Creature),
        "reach-guard: the source must be an artifact CREATURE, or this proves \
         something narrower than the rulings describe: {core:?}"
    );

    // Nothing else is on the battlefield, so the ONLY legal payment is the source
    // itself via the artifact leg.
    let outcome = runner.activate(source, 0).pay_with(&[source]).resolve();
    let state = outcome.state();
    assert_eq!(
        state.objects[&source].zone,
        Zone::Graveyard,
        "the source satisfies the article-led artifact leg and may pay with itself"
    );
}

/// COVERAGE GUARD for `Another` on the LEFT (creature) leg.
///
/// The three tests above all pass even if `Another` is stripped from the creature
/// leg: two of them submit a DIFFERENT permanent as the payment, and the third
/// pays through the artifact leg, which never carried `Another` to begin with.
/// Nothing above discriminates the property, so this drives the one case that
/// does — Mold Folk alone on the battlefield as a plain, NON-artifact creature.
///
/// CR 601.2h: with no other permanent to sacrifice and the source excluded from
/// the only leg it could match, the cost is unpayable — "Unpayable costs can't
/// be paid" — so the activation cannot be completed, and CR 733.1 reverses the
/// entire action, leaving the board exactly as it was.
///
/// Revert-failing: remove `FilterProp::Another` from the creature leg and the
/// source satisfies that leg itself, the cost becomes payable, and the
/// activation below is accepted instead of rejected.
#[test]
fn mold_folk_cannot_sacrifice_itself_while_it_is_only_a_creature() {
    let mut scenario = GameScenario::new_n_player(2, 42);
    scenario.at_phase(Phase::PreCombatMain);
    let source = scenario
        .add_creature_from_oracle(P0, "Mold Folk", 1, 1, MOLD_FOLK)
        .id();
    scenario.with_mana_pool(
        P0,
        vec![ManaUnit::new(
            ManaType::Colorless,
            ObjectId(0),
            false,
            vec![],
        )],
    );
    let mut runner = scenario.build();

    let core = &runner.state().objects[&source].card_types.core_types;
    assert!(
        core.contains(&CoreType::Creature) && !core.contains(&CoreType::Artifact),
        "reach-guard: the source must be a plain creature, or this tests the \
         artifact leg instead of the creature leg: {core:?}"
    );

    // The mana is available and the ability is otherwise activatable, so the ONLY
    // thing that can refuse this is the sacrifice filter excluding the source.
    let result = runner.act(GameAction::ActivateAbility {
        source_id: source,
        ability_index: 0,
        payment_mode: engine::types::game_state::CastPaymentMode::Auto,
    });
    assert!(
        result.is_err(),
        "\"another\" must exclude the source from the creature leg, leaving the \
         cost unpayable with nothing else on the battlefield, got {result:?}"
    );

    let state = runner.state();
    assert_eq!(
        state.objects[&source].zone,
        Zone::Battlefield,
        "the refused activation must leave the source on the battlefield"
    );
    assert_eq!(
        state.objects[&source]
            .counters
            .get(&CounterType::Plus1Plus1)
            .copied()
            .unwrap_or(0),
        0,
        "the ability must not have resolved"
    );
}

const GARGADON: &str = "Suspend 10—{R}\nSacrifice an artifact, creature, or land: Remove a time counter from this card. Activate only if this card is suspended.";

fn suspended_gargadon() -> (GameScenario, ObjectId) {
    let mut scenario = GameScenario::new_n_player(2, 42);
    scenario.at_phase(Phase::PreCombatMain);
    let source = scenario
        .add_creature_to_exile(P0, "Greater Gargadon", 9, 7)
        .from_oracle_text_with_keywords(&["Suspend"], GARGADON)
        .id();
    scenario.with_counter(source, CounterType::Time, 2);
    (scenario, source)
}

fn gargadon_activation(runner: &GameRunner, source: ObjectId) -> GameAction {
    let card = &runner.state().objects[&source];
    assert_eq!(card.zone, Zone::Exile);
    assert_eq!(card.counters.get(&CounterType::Time), Some(&2));
    GameAction::ActivateAbility {
        source_id: source,
        ability_index: card
            .abilities
            .iter()
            .position(|a| matches!(*a.effect, Effect::RemoveCounter { .. }))
            .expect("printed counter-removal ability"),
        payment_mode: CastPaymentMode::Auto,
    }
}

fn assert_sacrifice_prompt(runner: &GameRunner, eligible: &[ObjectId]) {
    let WaitingFor::PayCost {
        player,
        kind: PayCostKind::Sacrifice,
        choices,
        min_count,
        count,
        ..
    } = &runner.state().waiting_for
    else {
        panic!(
            "expected a sacrifice choice, got {:?}",
            runner.state().waiting_for
        )
    };
    assert_eq!((*player, *min_count, *count), (P0, 1, 1));
    let mut actual = choices.clone();
    actual.sort();
    let mut expected = eligible.to_vec();
    expected.sort();
    assert_eq!(actual, expected);
}

fn finish_gargadon_payment(
    runner: &mut GameRunner,
    source: ObjectId,
    selected: ObjectId,
    battlefield: &[ObjectId],
) {
    let owner = runner.state().objects[&selected].owner;
    runner
        .act(GameAction::SelectCards {
            cards: vec![selected],
        })
        .unwrap();
    let state = runner.state();
    assert_eq!(state.objects[&selected].zone, Zone::Graveyard);
    assert!(state.players[owner.0 as usize]
        .graveyard
        .contains(&selected));
    for id in battlefield.iter().filter(|id| **id != selected) {
        assert_eq!(state.objects[id].zone, Zone::Battlefield);
    }
    assert_eq!(state.objects[&source].zone, Zone::Exile);
    assert_eq!(
        state.objects[&source].counters.get(&CounterType::Time),
        Some(&2)
    );
    assert_eq!(state.stack.len(), 1);
    let entry = state.stack.back().unwrap();
    assert_eq!((entry.source_id, entry.controller), (source, P0));
    assert!(matches!(
        entry.kind,
        StackEntryKind::ActivatedAbility { .. }
    ));
    assert!(matches!(state.waiting_for, WaitingFor::Priority { .. }));
    runner.resolve_top();
    assert!(runner.state().stack.is_empty());
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { .. }
    ));
}

#[test]
fn greater_gargadon_pays_with_any_one_of_its_three_printed_types() {
    let (mut scenario, source) = suspended_gargadon();
    let ring = scenario
        .add_artifact_from_oracle(P0, "Sol Ring", "{T}: Add {C}{C}.")
        .id();
    let bears = scenario.add_creature(P0, "Grizzly Bears", 2, 2).id();
    let mountain = scenario.add_basic_land(P0, ManaColor::Red);
    let thopter = scenario
        .add_creature(P0, "Ornithopter", 0, 2)
        .as_artifact_creature()
        .id();
    let enchantment = scenario
        .add_enchantment_from_oracle(P0, "Enchantment", "")
        .id();
    let opponent = scenario.add_creature(P1, "Opponent creature", 2, 2).id();
    let mut runner = scenario.build();
    let eligible = [ring, bears, mountain, thopter];
    for (id, types) in [
        (ring, vec![CoreType::Artifact]),
        (bears, vec![CoreType::Creature]),
        (mountain, vec![CoreType::Land]),
        (thopter, vec![CoreType::Artifact, CoreType::Creature]),
        (enchantment, vec![CoreType::Enchantment]),
        (opponent, vec![CoreType::Creature]),
    ] {
        let obj = &runner.state().objects[&id];
        assert_eq!(obj.zone, Zone::Battlefield);
        assert_eq!(obj.card_types.core_types.len(), types.len());
        assert!(types.iter().all(|t| obj.card_types.core_types.contains(t)));
    }
    runner.act(gargadon_activation(&runner, source)).unwrap();
    assert_sacrifice_prompt(&runner, &eligible);
    for cards in [
        vec![enchantment],
        vec![opponent],
        vec![],
        vec![ring, mountain],
    ] {
        let before = serde_json::to_value(runner.state()).unwrap();
        assert!(runner.act(GameAction::SelectCards { cards }).is_err());
        assert_eq!(serde_json::to_value(runner.state()).unwrap(), before);
    }
    for chosen in eligible {
        let mut paying = GameRunner::from_state(runner.state().clone());
        finish_gargadon_payment(
            &mut paying,
            source,
            chosen,
            &[ring, bears, mountain, thopter, enchantment, opponent],
        );
    }
}

#[test]
fn greater_gargadon_needs_only_one_matching_permanent() {
    for kind in [
        None,
        Some(CoreType::Artifact),
        Some(CoreType::Creature),
        Some(CoreType::Land),
    ] {
        let (mut scenario, source) = suspended_gargadon();
        let enchantment = scenario
            .add_enchantment_from_oracle(P0, "Enchantment", "")
            .id();
        let opponent = scenario.add_creature(P1, "Opponent creature", 2, 2).id();
        let selected = kind.map(|kind| match kind {
            CoreType::Artifact => scenario
                .add_artifact_from_oracle(P0, "Sol Ring", "{T}: Add {C}{C}.")
                .id(),
            CoreType::Creature => scenario.add_creature(P0, "Grizzly Bears", 2, 2).id(),
            CoreType::Land => scenario.add_basic_land(P0, ManaColor::Red),
            _ => unreachable!(),
        });
        let mut runner = scenario.build();
        let action = gargadon_activation(&runner, source);
        if let Some(selected) = selected {
            runner.act(action).unwrap();
            assert_sacrifice_prompt(&runner, &[selected]);
            finish_gargadon_payment(
                &mut runner,
                source,
                selected,
                &[selected, opponent, enchantment],
            );
        } else {
            assert!(runner.act(action).is_err());
            assert!(runner.state().stack.is_empty());
            assert_eq!(runner.state().objects[&source].zone, Zone::Exile);
            assert_eq!(runner.state().objects[&enchantment].zone, Zone::Battlefield);
            assert_eq!(runner.state().objects[&opponent].zone, Zone::Battlefield);
        }
    }
}

#[test]
fn greater_gargadon_sacrifices_a_controlled_permanent_to_its_owners_graveyard() {
    let (mut scenario, source) = suspended_gargadon();
    let theirs = scenario.add_creature(P1, "Their creature", 2, 2).id();
    let ours = scenario.add_creature(P0, "Our creature", 2, 2).id();
    let mut runner = scenario.build();
    for (id, controller) in [(theirs, P0), (ours, P1)] {
        let object = runner.state_mut().objects.get_mut(&id).unwrap();
        object.base_controller = Some(controller);
        object.controller = controller;
    }
    runner.act(gargadon_activation(&runner, source)).unwrap();
    assert_sacrifice_prompt(&runner, &[theirs]);
    assert_eq!(runner.state().objects[&theirs].owner, P1);
    finish_gargadon_payment(&mut runner, source, theirs, &[ours, theirs]);
}
