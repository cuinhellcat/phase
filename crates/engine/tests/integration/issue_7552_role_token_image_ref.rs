//! #7552: a Role token's catalog `token_image_ref` must survive an UNLISTED
//! source card. Every Role exists on two flip sheets, so the preset scan finds
//! two semantically identical candidates; the source gate used to turn that
//! into a silent `None`, stranding the display on a name search no printing
//! satisfies (the engine names Roles "<Role> Role", printings are titled by the
//! bare face — CR 111.10 / `role_normalized_display_name`).
//!
//! NOT proven here: the ambiguity protection for semantically DIFFERENT
//! body-matches (that `None` path is preserved by `semantically_unique_ref`'s
//! shape, and no real catalog body exercises it end to end).

use engine::game::scenario::{GameScenario, P0};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaType, ManaUnit};
use engine::types::phase::Phase;

fn pool(n: usize) -> Vec<ManaUnit> {
    (0..n)
        .map(|_| ManaUnit::new(ManaType::Colorless, ObjectId(0), false, vec![]))
        .collect()
}

#[test]
fn a_role_from_an_unlisted_source_still_carries_its_catalog_image_ref() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(P0, pool(4));
    let host = scenario.add_creature(P0, "Chosen Host", 2, 2).id();
    let caster = scenario
        .add_creature_to_hand_from_oracle(
            P0,
            "Wicked Bard",
            1,
            1,
            "When this creature enters, create a Wicked Role token attached to target creature.",
        )
        .id();
    let mut runner = scenario.build();
    runner.cast(caster).target_object(host).resolve();
    runner.advance_until_stack_empty();

    let role = runner
        .state()
        .battlefield
        .iter()
        .find(|id| {
            runner.state().objects[id]
                .card_types
                .subtypes
                .iter()
                .any(|sub| sub == "Role")
        })
        .copied()
        .expect("the Wicked Role token exists");
    let obj = &runner.state().objects[&role];
    assert!(
        obj.token_image_ref.is_some(),
        "the catalog carries a Wicked image ref; the created token must too \
         (name={:?}, colors={:?}, subtypes={:?})",
        obj.name,
        obj.color,
        obj.card_types.subtypes
    );
}

/// The positive gate: a source the catalog DOES list keeps resolving exactly as
/// before — this row is what keeps the fallback from being the whole mechanism.
#[test]
fn a_role_from_a_listed_source_resolves_its_image_ref() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(P0, pool(4));
    let host = scenario.add_creature(P0, "Chosen Host", 2, 2).id();
    // "Monstrous Rage" is in the Monster preset's `source_card_names`.
    let caster = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Monstrous Rage",
            true,
            "Target creature gets +2/+0 until end of turn. Create a Monster Role token attached to it.",
        )
        .id();
    let mut runner = scenario.build();
    runner.cast(caster).target_object(host).resolve();
    runner.advance_until_stack_empty();

    let role = runner
        .state()
        .battlefield
        .iter()
        .find(|id| {
            runner.state().objects[id]
                .card_types
                .subtypes
                .iter()
                .any(|sub| sub == "Role")
        })
        .copied()
        .expect("the Monster Role token exists");
    assert!(
        runner.state().objects[&role].token_image_ref.is_some(),
        "the listed-source path must keep resolving"
    );
}
