//! Regression: the any-color / any-type mana rider that follows a cast grant —
//! "You may play the exiled card for as long as it remains exiled, and you may
//! spend mana as though it were mana of any color to cast that spell" (Siphon
//! Insight), "If you cast a spell this way, mana of any type can be spent to
//! cast it" (Bloodsoaked Insight) — is a payment concession that applies only
//! to mana spent casting through the granted permission (CR 118.14 + CR
//! 609.4b), not an effect of its own.
//!
//! Bug: the rider chunk reached the catch-all `SpendManaAsAnyColor` branch of
//! `lower_imperative_clause` and became a sibling `GenericEffect` with a bare
//! board-wide static. Two consequences, both measured before the fix:
//!
//!   * its "you may" was promoted to `AbilityDefinition.optional`, so the
//!     engine paused resolution with a `WaitingFor::OptionalEffectChoice` after
//!     the dig — a prompt for a choice the card never offers;
//!   * the granted permission was recorded with `mana_spend_permission: None`
//!     and no cast-time payment check consults the transient static, so the
//!     exiled card could not be paid for with off-color mana at all
//!     (`CastSpell` → `ActionNotAllowed("Cannot pay mana cost")`).
//!
//! Fix: `try_parse_mana_spend_rider` recognizes the rider and, when the clause
//! it follows grants a cast without a concession, emits it as
//! `PriorModifier::ManaSpendPermission` — folded onto that grant's
//! `mana_spend_permission` by `attach_mana_spend_permission_to_prior_cast_grant`.
//! Where the conjunct stays inside the grant's own sentence, the inline
//! recognizers gained the objects they were missing ("… to cast it", the
//! comma before "and mana of any …") — Court of Locthwain, #8481, whose whole
//! sentence used to be swallowed by the catch-all with the grant.
//! The tests here drive the real resolution (`GameScenario` / `GameRunner` /
//! `GameAction`) end to end: no optional prompt, the recorded permission
//! carries the concession, and the granted card is actually cast with
//! off-color lands (or, for the monarch, for free).

use engine::ai_support::legal_actions;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{CastingPermission, ManaSpendPermission, TargetRef};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::zones::Zone;
use engine::types::ObjectId;

const SIPHON_INSIGHT: &str = "Look at the top two cards of target opponent's library. Exile one of \
them face down and put the other on the bottom of that library. You may play the exiled card for as \
long as it remains exiled, and you may spend mana as though it were mana of any color to cast that \
spell.";

const COURT_OF_LOCTHWAIN: &str = "When this enchantment enters, you become the monarch.\n\
At the beginning of your upkeep, exile the top card of target opponent's library. You may play that \
card for as long as it remains exiled, and mana of any type can be spent to cast it. If you're the \
monarch, until end of turn, you may cast a spell from among cards exiled with this enchantment \
without paying its mana cost.";

const BLOODSOAKED_INSIGHT: &str = "Target opponent exiles the top three cards of their library. Until \
the end of your next turn, you may play those cards. If you cast a spell this way, mana of any type \
can be spent to cast it.";

/// What the drive saw: every prompt kind it answered, in order, plus the cards
/// the dig offered. The prompt list is the reach guard — the dig step proves
/// the grant resolved — and the discriminator: pre-fix an
/// `OptionalEffectChoice` sat between the dig and the return to priority.
struct Drive {
    prompts: Vec<&'static str>,
    dug: Vec<ObjectId>,
}

/// Cast `spell` (already free) and resolve it, answering only the prompts the
/// card's own text calls for. Any other prompt fails the test by name.
fn cast_and_resolve(runner: &mut GameRunner, spell: ObjectId) -> Drive {
    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("CastSpell accepted");
    settle(runner)
}

/// Answer the prompts a resolving grant calls for until the stack is empty.
fn settle(runner: &mut GameRunner) -> Drive {
    let mut drive = Drive {
        prompts: Vec::new(),
        dug: Vec::new(),
    };
    for _ in 0..40 {
        match runner.state().waiting_for.clone() {
            WaitingFor::OrderTriggers { triggers, .. } => {
                drive.prompts.push("OrderTriggers");
                let order = (0..triggers.len()).collect();
                runner
                    .act(GameAction::OrderTriggers { order })
                    .expect("OrderTriggers accepted");
            }
            WaitingFor::TriggerTargetSelection {
                target_slots,
                selection,
                ..
            }
            | WaitingFor::TargetSelection {
                target_slots,
                selection,
                ..
            } => {
                drive.prompts.push("TargetSelection");
                let choice = target_slots[selection.current_slot]
                    .legal_targets
                    .iter()
                    .find(|t| **t == TargetRef::Player(P1))
                    .cloned();
                runner
                    .act(GameAction::ChooseTarget { target: choice })
                    .expect("ChooseTarget accepted");
            }
            WaitingFor::DigChoice { cards, .. } => {
                drive.prompts.push("DigChoice");
                drive.dug = cards.clone();
                runner
                    .act(GameAction::SelectCards {
                        cards: vec![cards[0]],
                    })
                    .expect("SelectCards accepted");
            }
            WaitingFor::Priority { .. } => {
                if runner.state().stack.is_empty() {
                    return drive;
                }
                drive.prompts.push("Priority");
                runner
                    .act(GameAction::PassPriority)
                    .expect("PassPriority accepted");
            }
            other => panic!("unexpected prompt while resolving the grant: {other:?}"),
        }
    }
    panic!("the grant never resolved back to an empty stack");
}

/// The concession every permission recorded on `card` carries. Exact: a
/// permission without one is reported as `None`, so a half-stamped card fails.
fn recorded_concessions(runner: &GameRunner, card: ObjectId) -> Vec<Option<ManaSpendPermission>> {
    runner.state().objects[&card]
        .casting_permissions
        .iter()
        .map(|permission| match permission {
            CastingPermission::ExileWithAltCost {
                mana_spend_permission,
                ..
            }
            | CastingPermission::PlayFromExile {
                mana_spend_permission,
                ..
            } => *mana_spend_permission,
            other => panic!("unexpected permission recorded on the granted card: {other:?}"),
        })
        .collect()
}

/// Cast the granted `card` from exile paying with the caster's lands and let
/// it resolve. Returns how many of `lands` ended tapped.
fn cast_granted_card(runner: &mut GameRunner, card: ObjectId, lands: &[ObjectId]) -> usize {
    assert!(
        legal_actions(runner.state()).iter().any(
            |action| matches!(action, GameAction::CastSpell { object_id, .. } if *object_id == card)
        ),
        "the granted card must be offered as a legal cast with off-color lands"
    );
    let card_id = runner.state().objects[&card].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: card,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the granted card is cast with off-color lands");
    for _ in 0..10 {
        match runner.state().waiting_for.clone() {
            WaitingFor::Priority { .. } => {
                if runner.state().stack.is_empty() {
                    break;
                }
                runner
                    .act(GameAction::PassPriority)
                    .expect("PassPriority accepted");
            }
            other => panic!("unexpected prompt while the granted card resolves: {other:?}"),
        }
    }
    assert_eq!(
        runner.state().objects[&card].zone,
        Zone::Graveyard,
        "the granted sorcery resolved and went to its owner's graveyard"
    );
    lands
        .iter()
        .filter(|land| runner.state().objects[land].tapped)
        .count()
}

/// Siphon Insight: ", and you may spend mana as though it were mana of any
/// color to cast that spell" — the conjunct form, on a `CastFromZone` grant.
#[test]
fn siphon_insights_any_color_rider_is_scoped_to_the_exiled_card_and_asks_nothing() {
    let mut scenario = GameScenario::new_n_player(2, 42);
    scenario.at_phase(Phase::PreCombatMain);
    // P1's library, top first: a {G} sorcery over a filler card.
    let filler = scenario.add_card_to_library_top(P1, "Filler");
    let green = {
        let mut b = scenario.add_spell_to_library_top(P1, "Green Sorcery", false);
        b.with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Green],
            generic: 0,
        });
        b.id()
    };
    // P0 has only Swamps to pay with.
    let swamps = [
        scenario.add_basic_land(P0, ManaColor::Black),
        scenario.add_basic_land(P0, ManaColor::Black),
    ];
    let siphon = {
        let mut b =
            scenario.add_spell_to_hand_from_oracle(P0, "Siphon Insight", false, SIPHON_INSIGHT);
        b.with_mana_cost(ManaCost::default());
        b.id()
    };
    let mut runner = scenario.build();

    let drive = cast_and_resolve(&mut runner, siphon);

    // DISCRIMINATOR 1: the dig ran (reach guard) and NO optional-effect prompt
    // followed it. Pre-fix: ["Priority", "Priority", "DigChoice",
    // "OptionalEffectChoice"] — the last one panicked the drive by name.
    assert_eq!(
        drive.prompts,
        vec!["Priority", "Priority", "DigChoice"],
        "resolving Siphon Insight asks for the dig choice and nothing else"
    );
    assert_eq!(
        drive.dug,
        vec![green, filler],
        "the dig offered P1's top two cards"
    );
    assert_eq!(runner.state().objects[&green].zone, Zone::Exile);
    assert!(runner.state().objects[&green].face_down, "exiled face down");

    // DISCRIMINATOR 2: every permission recorded on the exiled card carries the
    // printed concession. Pre-fix both read `None`.
    assert_eq!(
        recorded_concessions(&runner, green),
        vec![
            Some(ManaSpendPermission::AnyColor),
            Some(ManaSpendPermission::AnyColor)
        ],
        "\"any color\" rides onto the granted permission as AnyColor"
    );

    // DISCRIMINATOR 3: the {G} card is cast with a Swamp. Pre-fix `CastSpell`
    // was refused with "Cannot pay mana cost".
    let tapped = cast_granted_card(&mut runner, green, &swamps);
    assert_eq!(tapped, 1, "exactly one Swamp paid for {{G}}");
}

/// Bloodsoaked Insight: "If you cast a spell this way, mana of any type can be
/// spent to cast it." — the separate-sentence form with the conditional
/// prefix, on a `GrantCastingPermission { PlayFromExile }` grant.
#[test]
fn bloodsoaked_insights_any_type_rider_is_scoped_to_the_exiled_cards() {
    let mut scenario = GameScenario::new_n_player(2, 42);
    scenario.at_phase(Phase::PreCombatMain);
    let deep = scenario.add_card_to_library_top(P1, "Deep");
    let third = scenario.add_card_to_library_top(P1, "Third");
    let second = scenario.add_card_to_library_top(P1, "Second");
    let green = {
        let mut b = scenario.add_spell_to_library_top(P1, "Green Sorcery", false);
        b.with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Green],
            generic: 0,
        });
        b.id()
    };
    let swamps = [
        scenario.add_basic_land(P0, ManaColor::Black),
        scenario.add_basic_land(P0, ManaColor::Black),
    ];
    let bloodsoaked = {
        let mut b = scenario.add_spell_to_hand_from_oracle(
            P0,
            "Bloodsoaked Insight",
            false,
            BLOODSOAKED_INSIGHT,
        );
        b.with_mana_cost(ManaCost::default());
        b.id()
    };
    let mut runner = scenario.build();

    let drive = cast_and_resolve(&mut runner, bloodsoaked);
    assert_eq!(
        drive.prompts,
        vec!["Priority", "Priority"],
        "resolving Bloodsoaked Insight asks nothing"
    );
    for card in [green, second, third] {
        assert_eq!(
            runner.state().objects[&card].zone,
            Zone::Exile,
            "top three exiled"
        );
    }
    assert_eq!(
        runner.state().objects[&deep].zone,
        Zone::Library,
        "the fourth card stays"
    );

    assert_eq!(
        recorded_concessions(&runner, green),
        vec![Some(ManaSpendPermission::AnyTypeOrColor)],
        "\"any type\" rides onto the granted permission as AnyTypeOrColor"
    );

    let tapped = cast_granted_card(&mut runner, green, &swamps);
    assert_eq!(tapped, 1, "exactly one Swamp paid for {{G}}");
}

/// Court of Locthwain (#8481): "You may play that card for as long as it
/// remains exiled, and mana of any type can be spent to cast it." — the inline
/// conjunct with "… to cast it". Pre-fix the whole sentence was captured by the
/// catch-all static and no permission existed at all: the exiled card was never
/// offered ("won't even give me the option to pay to cast"). Driven through P0's
/// own upkeep so the trigger, its player target, the exile, the recorded
/// permission and the cast are all real. `monarch` exercises the card's other
/// half — "If you're the monarch, until end of turn, you may cast a spell from
/// among cards exiled with this enchantment without paying its mana cost" —
/// which the report also names ("as the monarch it should be free to cast").
fn court_of_locthwain(monarch: bool) {
    let mut scenario = GameScenario::new_n_player(2, 42);
    scenario.at_phase(Phase::PreCombatMain);
    let deeper = scenario.add_card_to_library_top(P1, "Deeper");
    let green = {
        let mut b = scenario.add_spell_to_library_top(P1, "Green Sorcery", false);
        b.with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Green],
            generic: 0,
        });
        b.id()
    };
    // P1 draws once while their turn is crossed: a filler on top keeps the
    // {G} card as P1's top card at P0's upkeep. P0 draws from filler too.
    let p1_draw = scenario.add_card_to_library_top(P1, "P1 Draw");
    for _ in 0..4 {
        scenario.add_card_to_library_top(P0, "P0 Filler");
    }
    let swamps = [
        scenario.add_basic_land(P0, ManaColor::Black),
        scenario.add_basic_land(P0, ManaColor::Black),
    ];
    let court = scenario
        .add_enchantment_from_oracle(P0, "Court of Locthwain", COURT_OF_LOCTHWAIN)
        .id();
    let mut runner = scenario.build();
    if monarch {
        // Court's own ETB made its controller the monarch when it entered;
        // the enchantment starts on the battlefield here, so set the crown.
        runner.state_mut().monarch = Some(P0);
    }

    // Cross P1's turn into P0's next upkeep; the auto-advance stops at the
    // trigger's target prompt.
    runner.advance_to_phase(Phase::Upkeep);
    runner.advance_to_phase(Phase::PreCombatMain);
    runner.advance_to_phase(Phase::Upkeep);
    assert_eq!(runner.state().active_player, P0, "back in P0's turn");
    assert_eq!(runner.state().phase, Phase::Upkeep);
    // Reach guard: the upkeep trigger is on the stack (its single legal player
    // target, P1, was announced on the way).
    assert_eq!(
        runner.state().stack.len(),
        1,
        "Court's upkeep trigger is waiting to resolve"
    );
    assert_eq!(runner.state().stack[0].source_id, court);

    let drive = settle(&mut runner);
    // DISCRIMINATOR: no prompt beyond passing priority. Pre-fix the swallowed
    // sentence surfaced an `OptionalEffectChoice` here.
    assert_eq!(
        drive.prompts,
        vec!["Priority", "Priority"],
        "resolving the trigger asks nothing"
    );
    assert_eq!(
        runner.state().objects[&p1_draw].zone,
        Zone::Hand,
        "P1 drew the filler"
    );
    assert_eq!(
        runner.state().objects[&green].zone,
        Zone::Exile,
        "top card exiled"
    );
    assert_eq!(runner.state().objects[&deeper].zone, Zone::Library);

    // The paid permission carries the concession in both cases; the monarch's
    // free cast is a source-linked window (`ExiledBySource`, until end of
    // turn) that leaves no permission on the card — it shows in the cast
    // below, where no land is tapped.
    assert_eq!(
        recorded_concessions(&runner, green),
        vec![Some(ManaSpendPermission::AnyTypeOrColor)],
        "\"any type\" rides onto the granted permission"
    );
    // Move to P0's main phase and cast the {G} card.
    runner.advance_to_phase(Phase::PreCombatMain);
    assert_eq!(runner.state().active_player, P0);
    let untapped_before = swamps
        .iter()
        .filter(|land| !runner.state().objects[land].tapped)
        .count();
    assert_eq!(
        untapped_before, 2,
        "both Swamps untapped after P0's untap step"
    );
    let tapped = cast_granted_card(&mut runner, green, &swamps);
    if monarch {
        assert_eq!(tapped, 0, "the monarch casts for free — no Swamp tapped");
    } else {
        assert_eq!(tapped, 1, "exactly one Swamp paid for {{G}}");
    }
}

#[test]
fn court_of_locthwains_exiled_card_is_castable_with_any_mana() {
    court_of_locthwain(false);
}

#[test]
fn court_of_locthwains_exiled_card_is_free_for_the_monarch() {
    court_of_locthwain(true);
}
