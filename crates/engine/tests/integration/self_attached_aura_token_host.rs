//! CR 608.2k + CR 303.4: an Aura token created "attached to it" by an ability
//! that chose no target must enchant the object its trigger condition named.
//!
//! The class is every `Effect::Token` whose `attach_to` is `ParentTarget` inside
//! an ability with no target slot. Measured over the shipped pool: 8 such token
//! specs on 6 cards (Faunsbane Troll, Cursed Courtier, Unassuming Sage, Twisted
//! Sewer-Witch, Asinine Antics, and Gylwain's three modes), against 5 cards whose
//! identical phrase sits behind a target and always worked (Monstrous Rage,
//! Croaking Curse, Royal Treatment, Return Triumphant, Not Dead After All).
//!
//! `ParentTarget` reads the first object in `ability.targets`, and these
//! abilities put none there: the host resolved to `None`, the Role entered
//! enchanting nothing, the CR 704.5m unattached-Aura state-based action moved it
//! to the graveyard, and CR 111.7 ended it there — all inside one resolution, so
//! the card looked like it did nothing at all.
//!
//! The rows are built from Oracle text rather than the card export so they run
//! in CI, where only the curated fixture exists; the two shipped cards are then
//! replayed end to end behind the fixture guard this suite uses elsewhere.

use engine::game::game_object::AttachTarget;
use engine::game::scenario::{GameScenario, P0, P1};
use engine::game::scenario_db::GameScenarioDbExt;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

use crate::support::shared_card_db as load_db;

/// Faunsbane Troll's and Cursed Courtier's shared shape, with the reminder text
/// dropped: the enters trigger, the Role, and the bare pronoun host.
const SELF_ATTACHED_ETB: &str =
    "When this creature enters, create a Monster Role token attached to it.";

/// The same sentence with a target in front of it — the control for the
/// discriminator, and the shape Monstrous Rage and Royal Treatment print.
const TARGETED_ETB: &str = "When this creature enters, create a Monster Role token attached to \
     target creature you control.";

fn colorless(count: usize) -> Vec<ManaUnit> {
    pool(count, &[])
}

/// `generic` colorless units plus one unit of each named colour, which is what
/// the shipped cards' costs need ({2}{B}{G}, {2}{W}, {2}{U}{U}).
fn pool(generic: usize, colors: &[ManaType]) -> Vec<ManaUnit> {
    (0..generic)
        .map(|_| ManaType::Colorless)
        .chain(colors.iter().copied())
        .map(|kind| ManaUnit::new(kind, ObjectId(0), false, vec![]))
        .collect()
}

fn role_tokens(
    runner: &engine::game::scenario::GameRunner,
) -> Vec<&engine::game::game_object::GameObject> {
    runner
        .state()
        .objects
        .values()
        .filter(|object| object.card_types.subtypes.iter().any(|s| s == "Role"))
        .collect()
}

/// Searched across every zone on purpose: the defect's signature is a token that
/// reached the battlefield and was swept, which a battlefield-only query cannot
/// tell apart from one that was never created.
fn only_role_token(
    runner: &engine::game::scenario::GameRunner,
) -> &engine::game::game_object::GameObject {
    let tokens = role_tokens(runner);
    assert_eq!(tokens.len(), 1, "exactly one Role token per resolution");
    tokens[0]
}

#[test]
fn untargeted_role_token_enchants_the_creature_that_entered() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(P0, colorless(4));
    let subject = scenario
        .add_creature_to_hand_from_oracle(P0, "Self Role Host", 4, 4, SELF_ATTACHED_ETB)
        .id();
    let mut runner = scenario.build();

    runner.cast(subject).resolve();
    runner.advance_until_stack_empty();

    let token = only_role_token(&runner);
    assert_eq!(
        token.zone,
        Zone::Battlefield,
        "the Role must survive the CR 704.5m unattached-Aura check"
    );
    assert_eq!(
        token.attached_to,
        Some(AttachTarget::Object(subject)),
        "CR 608.2k: the pronoun names the object the trigger condition named"
    );
    assert!(
        runner.state().objects[&subject]
            .attachments
            .contains(&token.id),
        "both directions of the attachment relationship are written (CR 301.5)"
    );
}

/// The other side of the discriminator: with a target chosen, the host is the
/// target and the untargeted fallback must not outrank it.
#[test]
fn targeted_role_token_still_enchants_the_chosen_target() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(P0, colorless(4));
    let bystander = scenario.add_creature(P0, "Role Bystander", 2, 2).id();
    let subject = scenario
        .add_creature_to_hand_from_oracle(P0, "Targeted Role Host", 4, 4, TARGETED_ETB)
        .id();
    let mut runner = scenario.build();

    runner.cast(subject).target_object(bystander).resolve();
    runner.advance_until_stack_empty();

    let token = only_role_token(&runner);
    assert_eq!(token.zone, Zone::Battlefield);
    assert_eq!(
        token.attached_to,
        Some(AttachTarget::Object(bystander)),
        "the chosen target stays the host, not the creature that entered"
    );
}

/// End-to-end replay on the two shipped cards that reported the bug, so the fix
/// is pinned against real printed text and two different Roles rather than one
/// synthetic sentence. Skips where only the curated fixture is available; the
/// rows above carry the same claim in CI.
#[test]
fn shipped_self_attached_role_cards_keep_their_role() {
    let Some(db) = load_db() else {
        return;
    };

    for (card, cost, role) in [
        (
            "Faunsbane Troll",
            pool(2, &[ManaType::Black, ManaType::Green]),
            "Monster Role",
        ),
        (
            "Cursed Courtier",
            pool(2, &[ManaType::White]),
            "Cursed Role",
        ),
    ] {
        if db.get_face_by_name(card).is_none() {
            eprintln!("skipping: {card} is not in integration_cards.json.gz");
            continue;
        }
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        scenario.with_mana_pool(P0, cost);
        let subject = scenario.add_real_card(P0, card, Zone::Hand, db);
        let mut runner = scenario.build();
        engine::game::rehydrate_game_from_card_db(runner.state_mut(), db);

        runner.cast(subject).resolve();
        runner.advance_until_stack_empty();

        let token = only_role_token(&runner);
        assert_eq!(token.name, role, "{card} creates its own printed Role");
        assert_eq!(token.zone, Zone::Battlefield, "{card}'s Role stays in play");
        assert_eq!(
            token.attached_to,
            Some(AttachTarget::Object(subject)),
            "{card}'s Role enchants the creature that entered"
        );
    }
}

/// The loop case, which shares the same `ParentTarget` host but binds it per
/// iteration: Asinine Antics enchants every creature its opponents control. Each
/// Role must land on its own iteration host — a fallback that outranked the
/// loop's rebind would pile them all onto one permanent.
#[test]
fn for_each_role_tokens_enchant_their_own_iteration_host() {
    let Some(db) = load_db() else {
        return;
    };
    if db.get_face_by_name("Asinine Antics").is_none() {
        eprintln!("skipping: Asinine Antics is not in integration_cards.json.gz");
        return;
    }

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(P0, pool(2, &[ManaType::Blue, ManaType::Blue]));
    let first = scenario.add_creature(P1, "Antic Victim One", 2, 2).id();
    let second = scenario.add_creature(P1, "Antic Victim Two", 3, 3).id();
    let antics = scenario.add_real_card(P0, "Asinine Antics", Zone::Hand, db);
    let mut runner = scenario.build();
    engine::game::rehydrate_game_from_card_db(runner.state_mut(), db);

    runner.cast(antics).resolve();
    runner.advance_until_stack_empty();

    let mut hosts: Vec<u64> = role_tokens(&runner)
        .iter()
        .map(|token| {
            assert_eq!(
                token.zone,
                Zone::Battlefield,
                "every Role of the loop survives the unattached-Aura check"
            );
            match token.attached_to {
                Some(AttachTarget::Object(id)) => id.0,
                other => panic!("a loop Role must have an object host, got {other:?}"),
            }
        })
        .collect();
    hosts.sort_unstable();
    let mut expected = vec![first.0, second.0];
    expected.sort_unstable();
    assert_eq!(
        hosts, expected,
        "one Role per opponent creature, each on its own iteration host"
    );
}
