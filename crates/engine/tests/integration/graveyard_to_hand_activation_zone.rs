//! CR 113.6m — an activation zone is derived from a self-`ChangeZone`'s ORIGIN,
//! never gated on its destination.
//!
//! CR 113.6m: "An ability whose cost or effect specifies that it moves the
//! object it's on out of a particular zone functions only in that zone, unless
//! its trigger condition or a previous part of its cost or effect specifies
//! that the object is put into that zone or, if the object is an Aura, that the
//! object it enchants leaves the battlefield."
//!
//! The rule quantifies over the zone the object is moved *out of*. The
//! destination appears nowhere in it. `activation_zone_from_self_effect`
//! (`parser/oracle.rs`) was introduced for the `Hand → Battlefield` case (issue
//! #425, Talon Gates of Madara) and pinned the pattern to
//! `destination: Zone::Battlefield`, so the 55 abilities of the form "{cost}:
//! Return this card from your graveyard to your hand." derived no activation
//! zone at all. The runtime gate (`casting.rs`, `unwrap_or(Zone::Battlefield)`)
//! therefore offered them **on the battlefield** and withheld them **in the
//! graveyard** — both halves wrong.
//!
//! These tests drive the real `legal_actions` / `apply()` pipeline. The three
//! negatives each carry a positive reach-guard proving the ability could
//! otherwise have been offered — including that its **activation restrictions
//! are absent**, because `can_activate_ability_now` checks the zone *before* the
//! restrictions, so a restricted fixture would be rejected with and without the
//! fix and prove nothing.

use engine::ai_support::legal_actions;
use engine::game::game_object::AttachTarget;
use engine::game::layers::evaluate_layers;
use engine::game::sba::check_state_based_actions;
use engine::game::scenario::{GameScenario, P0};
use engine::types::ability::{ActivationRestriction, Effect, TargetFilter};
use engine::types::actions::GameAction;
use engine::types::identifiers::ObjectId;
use engine::types::keywords::Keyword;
use engine::types::mana::{ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Sanitarium Skeleton's entire printed Oracle text.
const SANITARIUM_SKELETON_TEXT: &str = "{2}{B}: Return this card from your graveyard to your hand.";

/// Bestial Bloodline's printed Oracle text (the reported card).
const BESTIAL_BLOODLINE_TEXT: &str = "Enchant creature\nEnchanted creature gets +2/+2.\n{4}{G}: Return this card from your graveyard to your hand.";

/// Slumbering Keepguard's printed Oracle text — the over-restriction canary.
const SLUMBERING_KEEPGUARD_TEXT: &str = "Whenever an enchantment you control enters, scry 1.\n{2}{W}: This creature gets +1/+1 until end of turn for each enchantment you control.";

/// Braided Net's printed Oracle text — the Craft hard constraint.
const BRAIDED_NET_TEXT: &str = "This artifact enters with three net counters on it.\n{T}, Remove a net counter from this artifact: Tap another target nonland permanent. Its activated abilities can't be activated for as long as it remains tapped.\nCraft with artifact {1}{U}";

fn floating(mana: &[ManaType]) -> Vec<ManaUnit> {
    mana.iter()
        .map(|t| ManaUnit::new(*t, ObjectId(0), false, vec![]))
        .collect()
}

/// Is `object`'s ability offered as an `ActivateAbility` action right now?
fn offers_activation(state: &engine::types::game_state::GameState, object: ObjectId) -> bool {
    legal_actions(state).iter().any(|action| {
        matches!(action, GameAction::ActivateAbility { source_id, .. } if *source_id == object)
    })
}

/// V10 — the bug, primary: a `Graveyard → Hand` self-return must NOT be offered
/// while the card sits on the battlefield, and submitting it must be rejected.
///
/// Sanitarium Skeleton is deliberately a plain `Creature — Skeleton` with a
/// single mana-cost ability and **no activation restrictions**: no Aura subtype
/// (so no CR 704.5m interaction) and nothing upstream of the zone gate that
/// could make this negative pass for the wrong reason.
#[test]
fn sanitarium_skeleton_not_activatable_from_battlefield() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let skeleton = scenario
        .add_creature(P0, "Sanitarium Skeleton", 1, 2)
        .with_subtypes(vec!["Skeleton"])
        .from_oracle_text(SANITARIUM_SKELETON_TEXT)
        .id();
    scenario.with_mana_pool(
        P0,
        floating(&[ManaType::Colorless, ManaType::Colorless, ManaType::Black]),
    );
    let mut runner = scenario.build();

    // ---- positive reach-guards: everything except the zone gate says yes ----
    let object = &runner.state().objects[&skeleton];
    assert_eq!(object.abilities.len(), 1, "one activated ability");
    let ability = &object.abilities[0];
    assert!(
        matches!(
            *ability.effect,
            Effect::ChangeZone {
                origin: Some(Zone::Graveyard),
                destination: Zone::Hand,
                target: TargetFilter::SelfRef,
                ..
            }
        ),
        "reach-guard: expected a Graveyard → Hand self-ChangeZone, got {:?}",
        ability.effect
    );
    assert_eq!(
        ability.activation_zone,
        Some(Zone::Graveyard),
        "CR 113.6m: the parser must derive Graveyard from the effect's origin"
    );
    assert!(
        ability.activation_restrictions.is_empty(),
        "reach-guard: no activation restriction may explain the negative — \
         `can_activate_ability_now` checks the zone BEFORE the restrictions, so \
         a restricted fixture would be rejected with and without the fix"
    );
    assert_eq!(
        object.zone,
        Zone::Battlefield,
        "reach-guard: the card is on the battlefield"
    );
    assert_eq!(
        runner.state().players[0].mana_pool.mana.len(),
        3,
        "reach-guard: {{2}}{{B}} is floating, so affordability cannot be the \
         reason the ability is withheld"
    );

    // ---- the assertions that fail if the fix is reverted ----
    assert!(
        !offers_activation(runner.state(), skeleton),
        "CR 113.6m: a graveyard self-return must not be offered from the \
         battlefield; legal_actions returned {:?}",
        legal_actions(runner.state())
    );
    assert!(
        runner
            .act(GameAction::ActivateAbility {
                source_id: skeleton,
                ability_index: 0,
            })
            .is_err(),
        "the submit path must reject the activation too (casting.rs zone gate)"
    );
}

/// V11 — the reported bug, verbatim: an attached Bestial Bloodline Aura on the
/// battlefield must not offer its graveyard-return ability. This mirrors the
/// user's saved game, where two attached Bestial Bloodline Auras were offering
/// the ability from the battlefield.
///
/// The Aura is attached with the repo's printed-Aura idiom (set `attached_to`,
/// push to the host's `attachments`, then `evaluate_layers`) — NOT
/// `attach_as_bestowed_aura`, which stamps `bestow_form` and would make the
/// fixture survive CR 704.5m for the wrong reason.
#[test]
fn attached_bestial_bloodline_aura_not_activatable_from_battlefield() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let host = scenario
        .add_creature(P0, "Savior of the Sleeping", 2, 3)
        .with_subtypes(vec!["Human", "Knight"])
        .id();
    // Re-parse after the Aura subtype is set and with MTGJSON's printed
    // `keywords: ["Enchant"]` hint, exactly as the card-data pipeline does —
    // `Keyword::Enchant(filter)` comes from that hint, and CR 704.5m's
    // `is_valid_attachment_target` keys on it.
    let aura = scenario
        .add_enchantment_from_oracle(P0, "Bestial Bloodline", BESTIAL_BLOODLINE_TEXT)
        .with_subtypes(vec!["Aura"])
        .from_oracle_text_with_keywords(&["Enchant"], BESTIAL_BLOODLINE_TEXT)
        .id();
    scenario.with_mana_pool(
        P0,
        floating(&[
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Green,
        ]),
    );
    let mut runner = scenario.build();

    // Attach the Aura the way 29 existing integration tests do.
    runner
        .state_mut()
        .objects
        .get_mut(&aura)
        .expect("the Aura exists")
        .attached_to = Some(AttachTarget::Object(host));
    runner
        .state_mut()
        .objects
        .get_mut(&host)
        .expect("the host exists")
        .attachments
        .push(aura);
    evaluate_layers(runner.state_mut());

    // ---- fixture-stability guard: CR 704.5m must not sweep the Aura away ----
    assert!(
        runner.state().objects[&aura]
            .keywords
            .iter()
            .any(|kw| matches!(kw, Keyword::Enchant(_))),
        "reach-guard: `is_valid_attachment_target` keys on Keyword::Enchant"
    );
    let mut events = Vec::new();
    check_state_based_actions(runner.state_mut(), &mut events);
    assert_eq!(
        runner.state().objects[&aura].zone,
        Zone::Battlefield,
        "CR 704.5m: the fixture Aura must survive the unattached-Aura SBA"
    );
    assert_eq!(
        runner.state().objects[&aura].attached_to,
        Some(AttachTarget::Object(host)),
        "CR 704.5m: the fixture Aura must still be attached after SBAs"
    );

    // ---- positive reach-guards ----
    let ability = &runner.state().objects[&aura].abilities[0];
    assert_eq!(ability.activation_zone, Some(Zone::Graveyard));
    assert!(
        ability.activation_restrictions.is_empty() && ability.condition.is_none(),
        "reach-guard: nothing downstream of the zone gate can explain the \
         negative (measured: both null on the printed card)"
    );

    // ---- the assertions that fail if the fix is reverted ----
    assert!(
        !offers_activation(runner.state(), aura),
        "CR 113.6m: the Aura's graveyard-return must not be offered while the \
         Aura is on the battlefield; legal_actions returned {:?}",
        legal_actions(runner.state())
    );
    assert!(
        runner
            .act(GameAction::ActivateAbility {
                source_id: aura,
                ability_index: 0,
            })
            .is_err(),
        "the submit path must reject the activation too"
    );
}

/// V12 — the other half: the same Aura sitting in the GRAVEYARD must be offered
/// the ability, and it must actually resolve the card into its owner's hand.
/// Pre-fix the activation was not offered at all.
///
/// No attachment is needed: `check_unattached_auras` iterates a **battlefield**
/// snapshot, so a graveyard card is never examined.
#[test]
fn bestial_bloodline_activatable_from_graveyard_returns_to_hand() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let aura = scenario
        .add_creature_to_graveyard(P0, "Bestial Bloodline", 0, 0)
        .as_enchantment()
        .with_subtypes(vec!["Aura"])
        .from_oracle_text(BESTIAL_BLOODLINE_TEXT)
        .id();
    scenario.with_mana_pool(
        P0,
        floating(&[
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Green,
        ]),
    );
    let mut runner = scenario.build();

    // ---- preconditions ----
    assert_eq!(
        runner.state().objects[&aura].zone,
        Zone::Graveyard,
        "precondition: the Aura starts in the graveyard"
    );
    let ability = &runner.state().objects[&aura].abilities[0];
    assert_eq!(ability.activation_zone, Some(Zone::Graveyard));
    assert!(
        ability.activation_restrictions.is_empty(),
        "precondition: no restriction can make this positive fail for a \
         non-zone reason"
    );

    // ---- the assertions that fail if the fix is reverted ----
    assert!(
        offers_activation(runner.state(), aura),
        "CR 113.6m: the ability must be offered from the graveyard; \
         legal_actions returned {:?}",
        legal_actions(runner.state())
    );
    let outcome = runner.activate(aura, 0).resolve();
    outcome.assert_zone(&[aura], Zone::Hand);
}

/// V13 — over-restriction canary. A battlefield ability with
/// `activation_zone: None` whose effect moves nothing must stay offered. This is
/// entry 25 of the user's saved game ("6 and 28 gone, 25 remains"), in the same
/// scenario shape as V11. It passes both before and after the fix by design.
#[test]
fn sibling_battlefield_pump_ability_still_offered() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let host = scenario
        .add_creature(P0, "Savior of the Sleeping", 2, 3)
        .with_subtypes(vec!["Human", "Knight"])
        .id();
    // Re-parse after the Aura subtype is set and with MTGJSON's printed
    // `keywords: ["Enchant"]` hint, exactly as the card-data pipeline does —
    // `Keyword::Enchant(filter)` comes from that hint, and CR 704.5m's
    // `is_valid_attachment_target` keys on it.
    let aura = scenario
        .add_enchantment_from_oracle(P0, "Bestial Bloodline", BESTIAL_BLOODLINE_TEXT)
        .with_subtypes(vec!["Aura"])
        .from_oracle_text_with_keywords(&["Enchant"], BESTIAL_BLOODLINE_TEXT)
        .id();
    let keepguard = scenario
        .add_creature(P0, "Slumbering Keepguard", 1, 1)
        .with_subtypes(vec!["Human", "Knight"])
        .from_oracle_text(SLUMBERING_KEEPGUARD_TEXT)
        .id();
    scenario.with_mana_pool(
        P0,
        floating(&[
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::Green,
            ManaType::Colorless,
            ManaType::Colorless,
            ManaType::White,
        ]),
    );
    let mut runner = scenario.build();
    runner
        .state_mut()
        .objects
        .get_mut(&aura)
        .expect("the Aura exists")
        .attached_to = Some(AttachTarget::Object(host));
    runner
        .state_mut()
        .objects
        .get_mut(&host)
        .expect("the host exists")
        .attachments
        .push(aura);
    evaluate_layers(runner.state_mut());

    // ---- reach-guards: the canary is offered for no reason but the default ----
    let ability = &runner.state().objects[&keepguard].abilities[0];
    assert_eq!(
        ability.activation_zone, None,
        "the pump ability states no zone and moves nothing, so CR 113.6's \
         battlefield default applies"
    );
    assert!(ability.activation_restrictions.is_empty());
    assert!(
        !matches!(*ability.effect, Effect::ChangeZone { .. }),
        "variant-agnostic: the derivation is never entered because the effect \
         is not a ChangeZone; got {:?}",
        ability.effect
    );

    assert!(
        offers_activation(runner.state(), keepguard),
        "the fix must not over-restrict: an ordinary battlefield ability stays \
         offered; legal_actions returned {:?}",
        legal_actions(runner.state())
    );
    assert!(
        !offers_activation(runner.state(), aura),
        "…while the Aura's graveyard-return in the same scenario is withheld"
    );
}

/// V9 — the Craft hard constraint, behaviorally. CR 702.167a: Craft's cost
/// exiles the permanent **from the battlefield**, so CR 113.6m's `unless` clause
/// exempts it and CR 113.6j makes the battlefield the only payable zone. The
/// synthesized Craft ability must keep `activation_zone: None` and must remain
/// activatable from the battlefield.
///
/// The reach-guards are load-bearing in both directions: the Craft ability
/// carries `AsSorcery`, so the fixture sits at a main phase with an empty stack
/// and the active player holding priority — otherwise a pass/fail here would be
/// about timing rather than about the zone.
#[test]
fn craft_ability_still_activatable_from_battlefield() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let net = scenario
        .add_creature(P0, "Braided Net", 0, 0)
        .as_artifact()
        .from_oracle_text(BRAIDED_NET_TEXT)
        .id();
    // A second artifact to exile as craft material.
    scenario
        .add_creature(P0, "Bone Saw", 0, 0)
        .as_artifact()
        .id();
    scenario.with_mana_pool(P0, floating(&[ManaType::Colorless, ManaType::Blue]));
    let runner = scenario.build();

    let object = &runner.state().objects[&net];
    assert_eq!(
        object.zone,
        Zone::Battlefield,
        "reach-guard: the Craft source is on the battlefield"
    );
    let craft = object
        .abilities
        .get(1)
        .expect("synthesize_craft adds the craft ability at index 1");
    assert_eq!(
        craft.activation_zone, None,
        "CR 702.167a + CR 113.6m: Craft functions from the battlefield — the \
         effect-side derivation must never stamp Exile here"
    );
    assert!(
        craft
            .activation_restrictions
            .contains(&ActivationRestriction::AsSorcery),
        "reach-guard: Craft is sorcery-speed, hence the main-phase fixture"
    );
    assert!(
        runner.state().stack.is_empty(),
        "reach-guard: sorcery-speed timing needs an empty stack"
    );

    let actions = legal_actions(runner.state());
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GameAction::ActivateAbility {
                source_id,
                ability_index: 1,
                ..
            } if *source_id == net
        )),
        "Craft must remain activatable from the battlefield; legal_actions \
         returned {actions:?}"
    );
}
