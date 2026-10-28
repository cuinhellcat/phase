//! Issue #9334 (rest), #9018 — "you may cast a <type> spell from <a graveyard>
//! this turn": a resolution-created permission for ONE cast.
//!
//! Chainer, Nightmare Adept; The Tomb of Aclazotz; Whispersteel Dagger. Each
//! lowered to an `Effect::CastFromZone` naming a class and no card, which
//! `cast_from_zone::resolve` cannot cast, so the ability granted nothing.
//!
//! CR 601.3: "A player can begin to cast a spell only if a rule or effect
//! allows that player to cast it and no rule or effect prohibits that player
//! from casting it." CR 611.2c: the permission modifies no object's
//! characteristics, so it also covers cards that reach the graveyard after it
//! began. The article ("a creature spell") makes it one cast.
//!
//! Cards are built from their verbatim Oracle text
//! (`client/public/card-data.json`); CI has no card database.

use engine::game::casting::{can_cast_object_now, spell_objects_available_to_cast};
use engine::game::combat::AttackTarget;
use engine::game::effects::attach::attach_to;
use engine::game::game_object::AttachTarget;
use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::mana::{ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const CHAINER: &str = "Discard a card: You may cast a creature spell from your graveyard this turn. Activate only once each turn.\nWhenever a nontoken creature you control enters, if you didn't cast it from your hand, it gains haste until your next turn.";

const TOMB_OF_ACLAZOTZ: &str = "(Transforms from Tarrian's Journal.)\n{T}: Add {B}.\n{T}: You may cast a creature spell from your graveyard this turn. If you do, it enters with a finality counter on it and is a Vampire in addition to its other types. (If a creature with a finality counter on it would die, exile it instead.)";

const WHISPERSTEEL_DAGGER: &str = "Equipped creature gets +2/+0.\nWhenever equipped creature deals combat damage to a player, you may cast a creature spell from that player's graveyard this turn, and you may spend mana as though it were mana of any color to cast that spell.\nEquip {3}";

fn mana(kind: ManaType, count: usize) -> Vec<ManaUnit> {
    let dummy = engine::types::identifiers::ObjectId(0);
    (0..count)
        .map(|_| ManaUnit::new(kind, dummy, false, vec![]))
        .collect()
}

fn colorless(count: usize) -> Vec<ManaUnit> {
    mana(ManaType::Colorless, count)
}

fn one_generic() -> ManaCost {
    ManaCost::Cost {
        shards: vec![],
        generic: 1,
    }
}

/// Chainer's grant admits a creature card in its controller's graveyard, and
/// only one: after the first cast the second creature card is no longer
/// castable. A noncreature card and a creature card in the opponent's
/// graveyard are never admitted.
#[test]
fn chainer_grants_one_creature_cast_from_own_graveyard() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let chainer = scenario
        .add_creature(P0, "Chainer, Nightmare Adept", 3, 2)
        .from_oracle_text(CHAINER)
        .id();
    let fodder = scenario.add_card_to_hand(P0, "Fodder");
    let first = scenario
        .add_creature_to_graveyard(P0, "First Bear", 2, 2)
        .with_mana_cost(one_generic())
        .id();
    let second = scenario
        .add_creature_to_graveyard(P0, "Second Bear", 2, 2)
        .with_mana_cost(one_generic())
        .id();
    let own_instant = scenario
        .add_spell_to_graveyard(P0, "Grave Instant", true)
        .with_mana_cost(one_generic())
        .id();
    let opponents_creature = scenario
        .add_creature_to_graveyard(P1, "Opponent's Bear", 2, 2)
        .with_mana_cost(one_generic())
        .id();
    scenario.with_mana_pool(P0, colorless(3));
    let mut runner = scenario.build();

    assert!(
        !can_cast_object_now(runner.state(), P0, first),
        "no graveyard creature is castable before Chainer's ability resolves"
    );

    runner.activate(chainer, 0).pay_with(&[fodder]).resolve();
    assert_eq!(runner.state().objects[&fodder].zone, Zone::Graveyard);

    let castable = spell_objects_available_to_cast(runner.state(), P0);
    assert!(
        castable.contains(&first) && castable.contains(&second),
        "both creature cards in P0's graveyard are offered: {castable:?}"
    );
    assert!(
        !castable.contains(&own_instant),
        "a noncreature card is not a creature spell"
    );
    assert!(
        !castable.contains(&opponents_creature),
        "\"your graveyard\" is P0's own (CR 404.1)"
    );

    let outcome = runner.cast(first).resolve();
    assert_eq!(outcome.zone_of(first), Zone::Battlefield);
    assert!(
        !can_cast_object_now(runner.state(), P0, second),
        "\"a creature spell\" is one cast: the grant is used up"
    );
}

/// The Tomb of Aclazotz: the creature cast through the grant enters with a
/// finality counter and is a Vampire in addition to its other types; the grant
/// allows one cast.
#[test]
fn tomb_of_aclazotz_cast_enters_with_finality_as_a_vampire() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let tomb = scenario
        .add_land_from_oracle(P0, "The Tomb of Aclazotz", TOMB_OF_ACLAZOTZ)
        .id();
    let first = scenario
        .add_creature_to_graveyard(P0, "First Bear", 2, 2)
        .with_mana_cost(one_generic())
        .id();
    let second = scenario
        .add_creature_to_graveyard(P0, "Second Bear", 2, 2)
        .with_mana_cost(one_generic())
        .id();
    scenario.with_mana_pool(P0, colorless(2));
    let mut runner = scenario.build();

    runner.activate(tomb, 1).resolve();
    assert!(
        can_cast_object_now(runner.state(), P0, first),
        "the Tomb's grant admits a creature card in P0's graveyard"
    );

    let outcome = runner.cast(first).resolve();
    assert_eq!(outcome.zone_of(first), Zone::Battlefield);
    assert_eq!(
        outcome.counters(first, CounterType::Finality),
        1,
        "\"it enters with a finality counter on it\""
    );
    assert!(
        runner.state().objects[&first]
            .card_types
            .subtypes
            .iter()
            .any(|subtype| subtype == "Vampire"),
        "\"is a Vampire in addition to its other types\""
    );
    assert!(
        !can_cast_object_now(runner.state(), P0, second),
        "the grant is one cast"
    );
}

/// A creature cast from hand while the Tomb's grant stands is neither a
/// Vampire nor given a finality counter: the rider rides the grant, so only the
/// spell cast through it — the graveyard creature cast next — receives it.
#[test]
fn tomb_rider_does_not_reach_a_cast_from_hand() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let tomb = scenario
        .add_land_from_oracle(P0, "The Tomb of Aclazotz", TOMB_OF_ACLAZOTZ)
        .id();
    let from_hand = scenario
        .add_creature_to_hand(P0, "Hand Bear", 2, 2)
        .with_mana_cost(one_generic())
        .id();
    let from_graveyard = scenario
        .add_creature_to_graveyard(P0, "Grave Bear", 2, 2)
        .with_mana_cost(one_generic())
        .id();
    scenario.with_mana_pool(P0, colorless(2));
    let mut runner = scenario.build();
    let is_vampire = |runner: &engine::game::scenario::GameRunner, id| {
        runner.state().objects[&id]
            .card_types
            .subtypes
            .iter()
            .any(|subtype| subtype == "Vampire")
    };

    runner.activate(tomb, 1).resolve();
    let outcome = runner.cast(from_hand).resolve();
    assert_eq!(outcome.zone_of(from_hand), Zone::Battlefield);
    assert_eq!(outcome.counters(from_hand, CounterType::Finality), 0);
    assert!(!is_vampire(&runner, from_hand));

    let outcome = runner.cast(from_graveyard).resolve();
    assert_eq!(outcome.zone_of(from_graveyard), Zone::Battlefield);
    assert_eq!(outcome.counters(from_graveyard, CounterType::Finality), 1);
    assert!(is_vampire(&runner, from_graveyard));
}

/// The board after an attacker carrying Whispersteel Dagger dealt combat damage
/// to P1, in P0's postcombat main phase with three red mana — enough to pay for
/// each {B} card in P1's graveyard under the concession, so mana never limits
/// how many are cast; whether a grant still stands decides it.
struct DaggerHit {
    runner: engine::game::scenario::GameRunner,
    /// Creature cards ({B}) in P1's graveyard.
    theirs: [engine::types::identifiers::ObjectId; 3],
    /// A creature card ({B}) in P0's own graveyard.
    mine: engine::types::identifiers::ObjectId,
    /// A creature card ({B}) in P0's hand.
    in_hand: engine::types::identifiers::ObjectId,
}

fn dagger_hits_p1(double_strike: bool) -> DaggerHit {
    let black_one_drop = ManaCost::Cost {
        shards: vec![ManaCostShard::Black],
        generic: 0,
    };
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let attacker = {
        let mut builder = scenario.add_creature(P0, "Attacker", 1, 1);
        if double_strike {
            builder.double_strike();
        }
        builder.id()
    };
    let dagger = scenario
        .add_artifact_from_oracle(P0, "Whispersteel Dagger", WHISPERSTEEL_DAGGER)
        .with_subtypes(vec!["Equipment"])
        .id();
    let theirs = ["Their Bear", "Their Second Bear", "Their Third Bear"].map(|name| {
        scenario
            .add_creature_to_graveyard(P1, name, 2, 2)
            .with_mana_cost(black_one_drop.clone())
            .id()
    });
    let mine = scenario
        .add_creature_to_graveyard(P0, "My Bear", 2, 2)
        .with_mana_cost(black_one_drop.clone())
        .id();
    let in_hand = scenario
        .add_creature_to_hand(P0, "Hand Bear", 2, 2)
        .with_mana_cost(black_one_drop)
        .id();
    let mut runner = scenario.build();
    attach_to(runner.state_mut(), dagger, attacker);
    assert_eq!(
        runner.state().objects[&dagger].attached_to,
        Some(AttachTarget::Object(attacker)),
        "the Dagger is attached to the attacker"
    );

    runner.advance_to_combat();
    runner
        .declare_attackers(&[(attacker, AttackTarget::Player(P1))])
        .expect("declare attackers");
    runner.combat_damage();
    runner.advance_to_phase(Phase::PostCombatMain);
    let pool = &mut runner.state_mut().players[0].mana_pool;
    pool.clear();
    for unit in mana(ManaType::Red, 3) {
        pool.add(unit);
    }
    DaggerHit {
        runner,
        theirs,
        mine,
        in_hand,
    }
}

/// Whispersteel Dagger: after the equipped creature deals combat damage to P1,
/// one creature card in P1's graveyard is castable, paid with off-color mana;
/// P0's own creature card is not, and nor is a second card after the cast.
#[test]
fn whispersteel_dagger_grants_one_cast_from_the_damaged_players_graveyard() {
    let DaggerHit {
        mut runner,
        theirs,
        mine,
        ..
    } = dagger_hits_p1(false);
    assert_eq!(runner.state().players[1].life, 17, "3 combat damage to P1");

    let castable = spell_objects_available_to_cast(runner.state(), P0);
    assert!(
        castable.contains(&theirs[0]),
        "a creature card in the damaged player's graveyard is castable: {castable:?}"
    );
    assert!(
        can_cast_object_now(runner.state(), P0, theirs[0]),
        "red mana prices {{B}} under the grant's concession"
    );
    assert!(
        !castable.contains(&mine),
        "\"that player's graveyard\" is P1's (CR 400.3), not P0's own"
    );

    // CR 609.4b: red mana pays the {B} of the spell cast through the grant.
    let outcome = runner.cast(theirs[0]).resolve();
    assert_eq!(outcome.zone_of(theirs[0]), Zone::Battlefield);
    assert_eq!(
        runner.state().objects[&theirs[0]].controller,
        P0,
        "the caster controls the spell and the permanent it becomes"
    );
    assert!(
        !can_cast_object_now(runner.state(), P0, theirs[1]),
        "\"a creature spell\" is one cast"
    );
}

/// The any-color concession belongs to the grant: with the same red mana, the
/// {B} card in P1's graveyard is castable and the {B} card in P0's hand is not,
/// until P0 has {B}.
#[test]
fn whispersteel_mana_concession_does_not_reach_a_cast_from_hand() {
    let DaggerHit {
        mut runner,
        theirs,
        in_hand,
        ..
    } = dagger_hits_p1(false);
    assert!(
        can_cast_object_now(runner.state(), P0, theirs[0]),
        "red mana pays {{B}} for the spell cast through the grant"
    );
    assert!(
        !can_cast_object_now(runner.state(), P0, in_hand),
        "red mana cannot pay {{B}} for a spell not cast through the grant"
    );
    for unit in mana(ManaType::Black, 1) {
        runner.state_mut().players[0].mana_pool.add(unit);
    }
    assert!(
        can_cast_object_now(runner.state(), P0, in_hand),
        "with {{B}} in the pool the hand card is castable"
    );
}

/// Double strike deals combat damage twice, so the ability triggers twice and
/// each resolution grants its own cast: two creature cards, not three.
#[test]
fn each_dagger_trigger_grants_its_own_cast() {
    let DaggerHit {
        mut runner, theirs, ..
    } = dagger_hits_p1(true);
    assert_eq!(runner.state().players[1].life, 14, "two hits of 3");

    // CR 601.2a: two grants admit the card, so the cast announces which one it
    // uses; each is its own permission.
    let card_id = runner.state().objects[&theirs[0]].card_id;
    let waiting = runner
        .act(GameAction::CastSpell {
            object_id: theirs[0],
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the graveyard cast starts")
        .waiting_for;
    let WaitingFor::CastingVariantChoice { options, .. } = waiting else {
        panic!("two grants ask which one the cast uses, got {waiting:?}");
    };
    assert_eq!(options.len(), 2, "one option per trigger's grant");
    runner
        .act(GameAction::ChooseCastingVariant { index: 0 })
        .expect("choosing an offered grant is legal");
    runner.resolve_top();
    assert_eq!(runner.state().objects[&theirs[0]].zone, Zone::Battlefield);

    let outcome = runner.cast(theirs[1]).resolve();
    assert_eq!(
        outcome.zone_of(theirs[1]),
        Zone::Battlefield,
        "the second grant still stands"
    );
    assert_eq!(
        runner.state().players[0].mana_pool.total(),
        1,
        "one red mana is left for a third {{B}} card"
    );
    assert!(
        !can_cast_object_now(runner.state(), P0, theirs[2]),
        "two triggers grant two casts"
    );
}

/// CR 514.2: an unused grant ends with the turn ("this turn").
#[test]
fn unused_chainer_grant_ends_with_the_turn() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let chainer = scenario
        .add_creature(P0, "Chainer, Nightmare Adept", 3, 2)
        .from_oracle_text(CHAINER)
        .id();
    let fodder = scenario.add_card_to_hand(P0, "Fodder");
    scenario.with_library_top(P0, &["Forest", "Forest"]);
    scenario.with_library_top(P1, &["Forest", "Forest"]);
    scenario
        .add_creature_to_graveyard(P0, "Grave Bear", 2, 2)
        .with_mana_cost(one_generic());
    let mut runner = scenario.build();

    runner.activate(chainer, 0).pay_with(&[fodder]).resolve();
    assert!(
        runner
            .state()
            .transient_continuous_effects
            .iter()
            .any(|effect| effect.source_id == chainer),
        "the grant stands this turn"
    );
    runner.advance_to_combat();
    runner.declare_attackers(&[]).expect("declare no attackers");
    runner.advance_to_end_step();
    let turn = runner.state().turn_number;
    runner.advance_to_phase(Phase::PreCombatMain);
    assert_eq!(runner.state().turn_number, turn + 1, "the next turn began");
    assert!(
        !runner
            .state()
            .transient_continuous_effects
            .iter()
            .any(|effect| effect.source_id == chainer),
        "the grant ended at cleanup"
    );
}

/// The Dagger's cast paid through the interactive mana-payment step (rather
/// than at announcement) is priced under the grant's concession too.
#[test]
fn whispersteel_concession_holds_for_manual_payment() {
    let DaggerHit {
        mut runner, theirs, ..
    } = dagger_hits_p1(false);
    let card_id = runner.state().objects[&theirs[0]].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: theirs[0],
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Manual,
        })
        .expect("the graveyard cast starts");
    assert!(
        matches!(runner.state().waiting_for, WaitingFor::ManaPayment { .. }),
        "manual payment opens the payment step, got {:?}",
        runner.state().waiting_for
    );
    // CR 118.3a: pinning a pool unit is judged while the cast is still pending.
    let pip_id = runner.state().players[0]
        .mana_pool
        .unit_at(0)
        .unwrap()
        .pip_id;
    runner
        .act(GameAction::SpendPoolMana { pip_id })
        .expect("red mana may be pinned to {B} under the grant");
    runner
        .act(GameAction::PassPriority)
        .expect("red mana pays {B} under the grant");
    assert_eq!(runner.state().objects[&theirs[0]].zone, Zone::Stack);
}

/// A printed static permission carrying `PermissionCastLimit::Single` has no
/// granting effect to end, so it is refused (fail closed); the same static as
/// `Unlimited` is offered.
#[test]
fn printed_single_cast_static_is_refused() {
    use engine::types::ability::CardPlayMode;
    use engine::types::ability::{
        ControllerRef, FilterProp, StaticDefinition, TargetFilter, TypeFilter, TypedFilter,
    };
    use engine::types::statics::{
        CastFrequency, GraveyardPermissionPool, PermissionCastLimit, StaticMode,
    };

    for (cast_limit, offered) in [
        (PermissionCastLimit::Unlimited, true),
        (PermissionCastLimit::Single, false),
    ] {
        let permission = StaticDefinition::new(StaticMode::GraveyardCastPermission {
            frequency: CastFrequency::Unlimited,
            play_mode: CardPlayMode::Cast,
            graveyard_destination_replacement: None,
            extra_cost: None,
            enters_with_counter: None,
            required_cast_keyword: None,
            pool: GraveyardPermissionPool::OwnGraveyard,
            cast_limit,
            mana_spend_permission: None,
            enters_modifications: Default::default(),
        })
        .affected(TargetFilter::Typed(
            TypedFilter::new(TypeFilter::Creature)
                .controller(ControllerRef::You)
                .properties(vec![FilterProp::InZone {
                    zone: Zone::Graveyard,
                }]),
        ));
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        scenario
            .add_creature(P0, "Permission Holder", 1, 1)
            .with_static_definition(permission);
        let bear = scenario
            .add_creature_to_graveyard(P0, "Grave Bear", 2, 2)
            .with_mana_cost(one_generic())
            .id();
        scenario.with_mana_pool(P0, colorless(1));
        let runner = scenario.build();
        assert_eq!(
            can_cast_object_now(runner.state(), P0, bear),
            offered,
            "{cast_limit:?}"
        );
    }
}
