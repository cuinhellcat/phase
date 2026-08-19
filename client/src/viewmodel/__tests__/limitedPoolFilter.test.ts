import { describe, expect, it } from "vitest";

import type {
  DraftCardInstance,
  DraftPoolGroup,
  DraftPoolGroupKind,
  DraftPoolGroups,
} from "../../adapter/draft-adapter";
import {
  axisKinds,
  EMPTY_POOL_FILTER,
  filterPool,
  poolFilterActive,
  toggleKind,
} from "../limitedPoolFilter";

// ── Fixtures: engine-shaped pool + groups (#7507) ───────────────────────

function card(name: string, id?: string): DraftCardInstance {
  return {
    instance_id: id ?? name,
    name,
    set_code: "TST",
    collector_number: "1",
    rarity: "common",
    colors: [],
    cmc: 2,
    type_line: "Creature",
  };
}

function group(
  kind: DraftPoolGroupKind,
  names: string[],
): DraftPoolGroup {
  return {
    kind,
    total: names.length,
    cards: names.map((name) => ({ card: card(name), count: 1 })),
  };
}

// A drafted pool with a duplicate: two copies of "Adept" as distinct
// instances, collapsed to one entry inside the engine groups — exactly the
// shape `sorted_entries` delivers.
const POOL: DraftCardInstance[] = [
  card("Adept", "adept-1"),
  card("Adept", "adept-2"),
  card("Bolt"),
  card("Charm"),
  card("Field"),
];

const GROUPS: DraftPoolGroups = {
  color_groups: [
    group("white", ["Adept"]),
    group("red", ["Bolt"]),
    group("multicolor", ["Charm"]),
    group("colorless", ["Field"]),
  ],
  type_groups: [
    group("creature", ["Adept"]),
    group("instant", ["Bolt"]),
    group("sorcery", ["Charm"]),
    group("land", ["Field"]),
  ],
  cmc_groups: [],
  rarity_groups: [
    group("rare", ["Charm"]),
    group("common", ["Adept", "Bolt", "Field"]),
  ],
  color_counts: { white: 2, blue: 0, black: 0, red: 1, green: 0 },
};

const names = (cards: DraftCardInstance[]) => cards.map((c) => c.instance_id);

describe("filterPool", () => {
  it("returns the listing unchanged when no filter is active", () => {
    expect(filterPool(POOL, GROUPS, EMPTY_POOL_FILTER)).toBe(POOL);
  });

  it("narrows by an engine type group, covering every duplicate copy", () => {
    // The group entry collapses the two Adept copies to one representative
    // instance; the name-keyed lookup must still keep BOTH instances.
    const result = filterPool(POOL, GROUPS, {
      ...EMPTY_POOL_FILTER,
      types: ["creature"],
    });
    expect(names(result)).toEqual(["adept-1", "adept-2"]);
  });

  it("ORs within an axis and ANDs across axes", () => {
    const within = filterPool(POOL, GROUPS, {
      ...EMPTY_POOL_FILTER,
      colors: ["red", "multicolor"],
    });
    expect(names(within)).toEqual(["Bolt", "Charm"]);

    const across = filterPool(POOL, GROUPS, {
      ...EMPTY_POOL_FILTER,
      colors: ["red", "multicolor"],
      rarities: ["rare"],
    });
    expect(names(across)).toEqual(["Charm"]);
  });

  it("applies the name query case-insensitively on top of the axes", () => {
    const result = filterPool(POOL, GROUPS, {
      ...EMPTY_POOL_FILTER,
      query: "aDePt",
      types: ["creature"],
    });
    expect(names(result)).toEqual(["adept-1", "adept-2"]);

    const excluded = filterPool(POOL, GROUPS, {
      ...EMPTY_POOL_FILTER,
      query: "bolt",
      types: ["creature"],
    });
    expect(excluded).toEqual([]);
  });

  it("drops a card absent from the axis groups when that axis is constrained", () => {
    // A listing entry the engine never classified (defensive: stale view)
    // must not slip through a constrained axis.
    const stray = card("Stray");
    const result = filterPool([...POOL, stray], GROUPS, {
      ...EMPTY_POOL_FILTER,
      rarities: ["common"],
    });
    expect(names(result)).toEqual(["adept-1", "adept-2", "Bolt", "Field"]);
  });
});

describe("toggleKind / poolFilterActive / axisKinds", () => {
  it("toggles a kind in and out", () => {
    expect(toggleKind([], "rare")).toEqual(["rare"]);
    expect(toggleKind(["rare"], "rare")).toEqual([]);
    expect(toggleKind(["rare"], "common")).toEqual(["rare", "common"]);
  });

  it("reports activity for any non-empty axis or query", () => {
    expect(poolFilterActive(EMPTY_POOL_FILTER)).toBe(false);
    expect(poolFilterActive({ ...EMPTY_POOL_FILTER, query: "  " })).toBe(false);
    expect(poolFilterActive({ ...EMPTY_POOL_FILTER, query: "a" })).toBe(true);
    expect(poolFilterActive({ ...EMPTY_POOL_FILTER, rarities: ["rare"] })).toBe(
      true,
    );
  });

  it("offers exactly the engine-delivered kinds in engine order", () => {
    expect(axisKinds(GROUPS.rarity_groups)).toEqual(["rare", "common"]);
  });
});
