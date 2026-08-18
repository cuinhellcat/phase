//! CR 303.4i: an Aura token whose host is undefined is NOT created (#7302).
//!
//! > If an effect attempts to put an Aura onto the battlefield attached to
//! > either an object or player it can't legally enchant or an object or player
//! > that is undefined, the Aura remains in its current zone. … If the Aura is a
//! > token, it isn't created.
//!
//! The engine used to create the token hostless and let the CR 704.5m
//! state-based action sweep it to a graveyard. That is observably different: the
//! token existed for a beat, fired enters-the-battlefield triggers, and left a
//! graveyard entry.
//!
//! Questing Cosplayer is the card that surfaced it, and it needed the parser
//! half too — "create a Questing Role token **and attach it to** target
//! creature" is the ACTION surface of the same CR 303.7 relation Oracle
//! otherwise prints as "…token **attached to** target creature", and only the
//! state surface was recognised, so the token was created with no host at all.
//!
//! The effect is built directly here rather than from a card: the negative case
//! needs an `attach_to` filter with NO bound target, which a card's own
//! targeting layer would never hand to the resolver.

use engine::game::scenario::{GameScenario, P0};
use engine::types::ability::{
    Effect, PtValue, QuantityExpr, ResolvedAbility, TargetFilter, TargetRef, TypeFilter,
    TypedFilter,
};
use engine::types::identifiers::{CardId, ObjectId};
use engine::types::zones::Zone;

/// A "Cursed Role" Aura token created attached to whatever `attach_to` names.
fn aura_token_effect(attach_to: Option<TargetFilter>) -> Effect {
    Effect::Token {
        name: "Cursed Role".to_string(),
        power: PtValue::Fixed(0),
        toughness: PtValue::Fixed(0),
        types: vec![
            "Enchantment".to_string(),
            "Aura".to_string(),
            "Role".to_string(),
        ],
        colors: vec![],
        keywords: vec![],
        tapped: false,
        count: QuantityExpr::Fixed { value: 1 },
        owner: TargetFilter::Controller,
        attach_to,
        enters_attacking: false,
        supertypes: vec![],
        static_abilities: vec![],
        enter_with_counters: vec![],
    }
}

fn creature_filter() -> TargetFilter {
    TargetFilter::Typed(TypedFilter {
        type_filters: vec![TypeFilter::Creature],
        controller: None,
        properties: Vec::new(),
    })
}

struct Board {
    runner: engine::game::scenario::GameRunner,
    source: ObjectId,
    host: ObjectId,
}

fn board() -> Board {
    let mut scenario = GameScenario::new();
    let host = scenario.add_creature(P0, "Host", 2, 2).id();
    let mut runner = scenario.build();
    let source = ObjectId(9001);
    runner.state_mut().objects.insert(
        source,
        engine::game::game_object::GameObject::new(
            source,
            CardId(9001),
            P0,
            "Token Source".to_string(),
            Zone::Battlefield,
        ),
    );
    Board {
        runner,
        source,
        host,
    }
}

fn resolve(board: &mut Board, effect: Effect, targets: Vec<TargetRef>) {
    let ability = ResolvedAbility::new(effect, targets, board.source, P0);
    let mut events = Vec::new();
    engine::game::effects::token::resolve(board.runner.state_mut(), &ability, &mut events)
        .expect("token effect resolves");
}

fn role_tokens(board: &Board, zone: Zone) -> Vec<ObjectId> {
    board
        .runner
        .state()
        .objects
        .values()
        .filter(|object| object.zone == zone && object.is_token && object.name.contains("Role"))
        .map(|object| object.id)
        .collect()
}

/// CR 303.4i: the instruction names a host, nothing binds it, so no token is
/// created — and, unlike the create-then-sweep path, nothing lands in a
/// graveyard either.
///
/// Reverting the guard turns this red: the token is created, the CR 704.5m SBA
/// moves it, and the graveyard assertion fails.
#[test]
fn an_aura_token_with_an_unbound_host_is_not_created() {
    let mut board = board();
    resolve(
        &mut board,
        aura_token_effect(Some(creature_filter())),
        vec![],
    );

    assert!(
        role_tokens(&board, Zone::Battlefield).is_empty(),
        "CR 303.4i: an Aura token with an undefined host is not created"
    );
    assert!(
        role_tokens(&board, Zone::Graveyard).is_empty(),
        "not created means it never reached a graveyard either"
    );
}

/// Positive counter-direction: a bound host still creates the token and attaches
/// it. This is also the reach guard — if this harness could not create an Aura
/// token at all, the negative assertion above would be vacuous.
#[test]
fn a_bound_host_still_gets_its_aura_token() {
    let mut board = board();
    let host = board.host;
    resolve(
        &mut board,
        aura_token_effect(Some(creature_filter())),
        vec![TargetRef::Object(host)],
    );

    let tokens = role_tokens(&board, Zone::Battlefield);
    assert_eq!(tokens.len(), 1, "the Role token must be created");
    assert_eq!(
        board.runner.state().objects[&tokens[0]]
            .attached_to
            .as_ref()
            .and_then(|attached| attached.as_object()),
        Some(host),
        "the Role token must enter attached to its host"
    );
}

/// The guard is scoped to Auras: an ordinary token that names no host is
/// untouched, so nothing about the common create-a-token path changes.
#[test]
fn an_ordinary_token_without_a_host_is_still_created() {
    let mut board = board();
    let before = board.runner.state().battlefield.len();
    resolve(&mut board, aura_token_effect(None), vec![]);

    assert_eq!(
        board.runner.state().battlefield.len(),
        before + 1,
        "a token that names no host is created as before"
    );
}
