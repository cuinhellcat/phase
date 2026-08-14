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
use engine::types::ability::{
    AbilityDefinition, AbilityKind, Effect, EffectScope, PtValue, QuantityExpr, TapStateChange,
    TargetFilter, TypedFilter,
};
use engine::types::events::GameEvent;
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

/// Twisted Sewer-Witch's and Asinine Antics' shape: the same host filter, bound
/// once per iteration instead of once per ability.
const FOR_EACH_ETB: &str = "When this creature enters, for each creature you control, create a \
     Wicked Role token attached to that creature.";

/// The hosts of every Role in play, sorted, so a row can compare sets without
/// depending on object-id order.
fn sorted_hosts(runner: &engine::game::scenario::GameRunner) -> Vec<u64> {
    let mut hosts: Vec<u64> = role_tokens(runner)
        .iter()
        .map(|token| {
            assert_eq!(
                token.zone,
                Zone::Battlefield,
                "every Role must survive the unattached-Aura check"
            );
            match token.attached_to {
                Some(AttachTarget::Object(id)) => id.0,
                other => panic!("a Role must have an object host, got {other:?}"),
            }
        })
        .collect();
    hosts.sort_unstable();
    hosts
}

fn sorted_ids(ids: &[ObjectId]) -> Vec<u64> {
    let mut ids: Vec<u64> = ids.iter().map(|id| id.0).collect();
    ids.sort_unstable();
    ids
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
        "CR 701.3a: the Role is attached to that creature; the reverse list is \
         the engine invariant that says so from the host's side"
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
/// iteration. Each Role must land on its own iteration host — a fallback that
/// outranked the loop's rebind would pile them all onto one permanent, or onto
/// the creature that entered.
///
/// Built from Oracle text so the claim is exercised in CI, then replayed on
/// Asinine Antics where the full export is available.
#[test]
fn for_each_role_tokens_enchant_their_own_iteration_host() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(P0, colorless(4));
    let buddy = scenario.add_creature(P0, "Loop Buddy", 1, 1).id();
    let subject = scenario
        .add_creature_to_hand_from_oracle(P0, "Loop Role Host", 3, 3, FOR_EACH_ETB)
        .id();
    let mut runner = scenario.build();

    runner.cast(subject).resolve();
    runner.advance_until_stack_empty();

    assert_eq!(
        sorted_hosts(&runner),
        sorted_ids(&[buddy, subject]),
        "one Role per creature, each on its own iteration host"
    );

    let Some(db) = load_db() else {
        return;
    };
    if db.get_face_by_name("Asinine Antics").is_none() {
        eprintln!(
            "skipping the shipped replay: Asinine Antics is not in integration_cards.json.gz"
        );
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

    assert_eq!(
        sorted_hosts(&runner),
        sorted_ids(&[first, second]),
        "Asinine Antics enchants each opponent creature separately"
    );
}

fn role_token_effect(attach_to: TargetFilter) -> Effect {
    Effect::Token {
        name: "Monster Role".to_string(),
        power: PtValue::Fixed(0),
        toughness: PtValue::Fixed(0),
        types: vec![
            "Enchantment".to_string(),
            "Aura".to_string(),
            "Role".to_string(),
        ],
        colors: Vec::new(),
        keywords: Vec::new(),
        tapped: false,
        count: QuantityExpr::Fixed { value: 1 },
        owner: TargetFilter::Controller,
        attach_to: Some(attach_to),
        enters_attacking: false,
        supertypes: Vec::new(),
        static_abilities: Vec::new(),
        enter_with_counters: Vec::new(),
    }
}

/// A permanent whose activated ability taps a chosen creature and then creates
/// a Role token "attached to" `attach_to` — one ability holding both a selected
/// object target and a host filter, which is the shape where the two can be
/// confused. Everything runs through the production path: activation, target
/// selection, stack resolution and state-based actions.
struct RoleChain {
    runner: engine::game::scenario::GameRunner,
    source: ObjectId,
    selected: ObjectId,
    partner: ObjectId,
}

impl RoleChain {
    fn build(attach_to: TargetFilter) -> Self {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let selected = scenario.add_creature(P0, "Selected Bystander", 2, 2).id();
        let partner = scenario.add_creature(P0, "Paired Partner", 2, 2).id();
        let source = scenario
            .add_creature(P0, "Chain Source", 2, 2)
            .with_ability_definition(AbilityDefinition {
                sub_ability: Some(Box::new(AbilityDefinition::new(
                    AbilityKind::Activated,
                    role_token_effect(attach_to),
                ))),
                ..AbilityDefinition::new(
                    AbilityKind::Activated,
                    Effect::SetTapState {
                        target: TargetFilter::Typed(TypedFilter::creature()),
                        scope: EffectScope::Single,
                        state: TapStateChange::Tap,
                    },
                )
            })
            .id();
        Self {
            runner: scenario.build(),
            source,
            selected,
            partner,
        }
    }

    /// Activates the ability on `selected`, settles the stack, and returns the
    /// Role token the resolution created.
    ///
    /// The creation is read from the resolution's own events, not from the
    /// final board: a Role that ends up with no host is unattached, the
    /// CR 704.5m state-based action moves it to the graveyard and CR 111.7 ends
    /// it there — so a board-only check could not tell "created and swept" from
    /// "never created", and the negative rows below would pass on a resolution
    /// that never reached the token clause at all.
    fn run(&mut self) -> ObjectId {
        let source = self.source;
        let selected = self.selected;
        let outcome = self
            .runner
            .activate(source, 0)
            .target_object(selected)
            .resolve();
        let created: Vec<ObjectId> = outcome
            .events()
            .iter()
            .filter_map(|event| match event {
                GameEvent::TokenCreated { object_id, .. } => Some(*object_id),
                _ => None,
            })
            .collect();
        self.runner.advance_until_stack_empty();
        assert!(
            self.runner.state().objects[&selected].tapped,
            "reach guard: the parent effect ran, so resolution reached the token clause"
        );
        assert_eq!(
            created.len(),
            1,
            "reach guard: exactly one Role token was created"
        );
        created[0]
    }

    /// The host the created token ended up with, or `None` where the token was
    /// left unattached (and therefore no longer exists).
    fn host_of(&self, token: ObjectId) -> Option<AttachTarget> {
        self.runner
            .state()
            .objects
            .get(&token)
            .and_then(|object| object.attached_to)
    }

    fn attachments_of(&self, object: ObjectId) -> usize {
        self.runner.state().objects[&object].attachments.len()
    }
}

#[test]
fn a_context_filter_host_never_falls_back_to_the_selected_target() {
    // CR 400.7j + CR 608.2k: "the object paid as a cost" names nothing here, so
    // the token gets no host — it must NOT inherit the chosen target.
    let mut chain = RoleChain::build(TargetFilter::CostPaidObject);
    let token = chain.run();
    assert_eq!(
        chain.attachments_of(chain.selected),
        0,
        "a reference filter that resolves to nothing must leave the selected \
         target unenchanted"
    );
    assert_eq!(
        chain.host_of(token),
        None,
        "no Role may be attached at all when its declared authority named no host"
    );

    // The discriminating half: a context filter that DOES name an object
    // resolves through its own authority, and that object is the host even
    // though a different object sits in the ability's target slot.
    let mut chain = RoleChain::build(TargetFilter::SelfRef);
    let token = chain.run();
    assert_eq!(
        chain.host_of(token),
        Some(AttachTarget::Object(chain.source)),
        "the filter's own authority names the host, not the selected target"
    );
    assert_eq!(chain.attachments_of(chain.selected), 0);
}

/// CR 702.95b: `SourceOrPaired` matches the source and the creature it is
/// paired with. `TargetFilter::is_context_ref` classifies it as an automatic
/// context reference — it is never a chosen target slot — so it must not read
/// `ability.targets`. It names two objects rather than one host, and no host
/// authority exists for that here, so it fails closed until one does.
#[test]
fn a_paired_source_filter_never_inherits_the_selected_target() {
    let mut chain = RoleChain::build(TargetFilter::SourceOrPaired);
    let (source, partner) = (chain.source, chain.partner);
    {
        let state = chain.runner.state_mut();
        state.objects.get_mut(&source).expect("source").paired_with = Some(partner);
        state
            .objects
            .get_mut(&partner)
            .expect("partner")
            .paired_with = Some(source);
    }
    let token = chain.run();

    assert_eq!(
        chain.attachments_of(chain.selected),
        0,
        "a paired-source reference must not enchant the object the ability selected"
    );
    assert_eq!(
        chain.host_of(token),
        None,
        "with no single host authority for the pair, the Role gets no host at all"
    );
}
