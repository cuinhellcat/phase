import type {
  DraftCardInstance,
  DraftPoolGroup,
  DraftPoolGroupKind,
  DraftPoolGroups,
} from "../adapter/draft-adapter";

// ── Limited build screen pool filters (#7507) ───────────────────────────
//
// The classification is ENGINE-OWNED: every axis keys on the groups
// `DraftPoolGroups::from_pool` already delivers (color / type / rarity), so the
// display never re-derives a game category from raw card fields (the #6663
// finding). Only the name query is display-layer work, mirroring the existing
// addable-cards search box on the same screen.

/** One axis selection; empty means "no constraint on this axis". */
export interface PoolFilterState {
  query: string;
  types: DraftPoolGroupKind[];
  colors: DraftPoolGroupKind[];
  rarities: DraftPoolGroupKind[];
}

export const EMPTY_POOL_FILTER: PoolFilterState = {
  query: "",
  types: [],
  colors: [],
  rarities: [],
};

export function poolFilterActive(filter: PoolFilterState): boolean {
  return (
    filter.query.trim() !== "" ||
    filter.types.length > 0 ||
    filter.colors.length > 0 ||
    filter.rarities.length > 0
  );
}

/** Toggle one kind within an axis selection. */
export function toggleKind(
  selected: DraftPoolGroupKind[],
  kind: DraftPoolGroupKind,
): DraftPoolGroupKind[] {
  return selected.includes(kind)
    ? selected.filter((k) => k !== kind)
    : [...selected, kind];
}

/**
 * Engine classification lookup: group member INSTANCE id → group kind.
 *
 * Keyed on the per-copy `instance_ids` the engine carries on every collapsed
 * entry — NOT on the name: same-name instances can sit in different groups on
 * one axis (a reprint at a different rarity), and a name-keyed map would let
 * one copy's classification overwrite the other's (#7546 review).
 */
function kindByInstanceId(
  groups: DraftPoolGroup[],
): Map<string, DraftPoolGroupKind> {
  const index = new Map<string, DraftPoolGroupKind>();
  for (const group of groups) {
    for (const entry of group.cards) {
      for (const id of entry.instance_ids) {
        index.set(id, group.kind);
      }
    }
  }
  return index;
}

function axisMatches(
  selected: DraftPoolGroupKind[],
  index: Map<string, DraftPoolGroupKind>,
  instanceId: string,
): boolean {
  if (selected.length === 0) return true;
  const kind = index.get(instanceId);
  return kind !== undefined && selected.includes(kind);
}

/**
 * Narrow a pool listing by the current filter: OR within an axis, AND across
 * axes, plus a case-insensitive name substring. The listing may be any subset
 * of the pool the groups were built from (the build screen passes the pool
 * minus the cards already moved to the deck).
 */
export function filterPool(
  pool: DraftCardInstance[],
  groups: DraftPoolGroups,
  filter: PoolFilterState,
): DraftCardInstance[] {
  if (!poolFilterActive(filter)) return pool;

  const query = filter.query.trim().toLowerCase();
  const typeIndex = kindByInstanceId(groups.type_groups);
  const colorIndex = kindByInstanceId(groups.color_groups);
  const rarityIndex = kindByInstanceId(groups.rarity_groups);

  return pool.filter(
    (card) =>
      (query === "" || card.name.toLowerCase().includes(query)) &&
      axisMatches(filter.types, typeIndex, card.instance_id) &&
      axisMatches(filter.colors, colorIndex, card.instance_id) &&
      axisMatches(filter.rarities, rarityIndex, card.instance_id),
  );
}

/**
 * The chips one axis offers: exactly the groups the engine delivered for this
 * pool, in engine order — no hand-kept kind list, no empty chips.
 */
export function axisKinds(groups: DraftPoolGroup[]): DraftPoolGroupKind[] {
  return groups.map((group) => group.kind);
}
