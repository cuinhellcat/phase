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

/** One collapsed entry: representative card plus the ids of every copy. */
function entryOf(name: string, ids?: string[]): DraftPoolGroup["cards"][number] {
  const instance_ids = ids ?? [name];
  return { card: card(name), count: instance_ids.length, instance_ids };
}

function group(
  kind: DraftPoolGroupKind,
  entries: Array<string | ReturnType<typeof entryOf>>,
): DraftPoolGroup {
  const cards = entries.map((e) => (typeof e === "string" ? entryOf(e) : e));
  return {
    kind,
    total: cards.reduce((sum, e) => sum + e.count, 0),
    cards,
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

const ADEPT_IDS = ["adept-1", "adept-2"];

const GROUPS: DraftPoolGroups = {
  color_groups: [
    group("white", [entryOf("Adept", ADEPT_IDS)]),
    group("red", ["Bolt"]),
    group("multicolor", ["Charm"]),
    group("colorless", ["Field"]),
  ],
  type_groups: [
    group("creature", [entryOf("Adept", ADEPT_IDS)]),
    group("instant", ["Bolt"]),
    group("sorcery", ["Charm"]),
    group("land", ["Field"]),
  ],
  cmc_groups: [],
  rarity_groups: [
    group("rare", ["Charm"]),
    group("common", [entryOf("Adept", ADEPT_IDS), entryOf("Bolt"), entryOf("Field")]),
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
    // card; its `instance_ids` must keep BOTH instances addressable.
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

describe("filterPool with same-name instances at different rarities", () => {
  // A reprint at a different rarity: the two copies share a NAME but sit in
  // different rarity groups. A name-keyed lookup lets one copy's
  // classification overwrite the other's and hides the wrong card
  // (#7546 review); the instance-keyed lookup keeps each copy its own.
  const pool = [card("Adept", "adept-common"), card("Adept", "adept-rare")];
  const groups: DraftPoolGroups = {
    color_groups: [],
    type_groups: [group("creature", [entryOf("Adept", ["adept-common", "adept-rare"])])],
    cmc_groups: [],
    rarity_groups: [
      group("rare", [entryOf("Adept", ["adept-rare"])]),
      group("common", [entryOf("Adept", ["adept-common"])]),
    ],
    color_counts: { white: 0, blue: 0, black: 0, red: 0, green: 0 },
  };

  it("keeps exactly the copy whose OWN rarity is selected", () => {
    const rare = filterPool(pool, groups, {
      ...EMPTY_POOL_FILTER,
      rarities: ["rare"],
    });
    expect(names(rare)).toEqual(["adept-rare"]);

    const common = filterPool(pool, groups, {
      ...EMPTY_POOL_FILTER,
      rarities: ["common"],
    });
    expect(names(common)).toEqual(["adept-common"]);
  });

  it("still covers both copies on their shared axis", () => {
    const result = filterPool(pool, groups, {
      ...EMPTY_POOL_FILTER,
      types: ["creature"],
    });
    expect(names(result)).toEqual(["adept-common", "adept-rare"]);
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
