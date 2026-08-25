import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { EngineAdapter, EngineSnapshot, GameState, LegalActionsResult } from "../../adapter/types";
import { nextSnapshotSeq } from "../../adapter/types";
import { useGameStore } from "../../stores/gameStore";
import { processRemoteUpdate } from "../dispatch";
import {
  WATCHDOG_ARM_DELAY_MS,
  createStaleStateWatchdog,
  resyncFromAdapter,
  stateFingerprint,
} from "../staleStateWatchdog";

// These tests prove the healing mechanism and its causality: each commit
// arms exactly one deferred check, a clean check disarms until the next
// commit (no polling), and a divergent check re-commits the adapter's
// snapshot. They do NOT prove the original pod-draft incident (a rejected
// delivery freezing the host on the mulligan overlay) end-to-end — that
// failure needs a real P2P match; the delivery `.catch` sites and the
// emit-before-AI-loop reorder in `p2p-adapter.ts` cover it by construction.

function stateAt(turn: number, priorityPlayer: number): GameState {
  return {
    turn_number: turn,
    active_player: 0,
    phase: "PreCombatMain",
    players: [],
    priority_player: priorityPlayer,
    objects: {},
    next_object_id: 1,
    battlefield: [],
    stack: [],
    exile: [],
    rng_seed: 42,
    combat: null,
    waiting_for: { type: "Priority", data: { player: priorityPlayer } },
    has_pending_cast: false,
    lands_played_this_turn: 0,
    max_lands_per_turn: 1,
    priority_pass_count: 0,
    pending_replacement: null,
    layers_dirty: false,
    next_timestamp: 1,
  };
}

function noLegalActions(): LegalActionsResult {
  return { actions: [], autoPassRecommended: false };
}

function snapshotOf(state: GameState): EngineSnapshot {
  return { state, legalResult: noLegalActions(), seq: nextSnapshotSeq() };
}

/** Adapter stub: only `getSnapshot` is consulted by the watchdog. */
function stubAdapter(current: () => EngineSnapshot): EngineAdapter {
  return { getSnapshot: async () => current(), dispose: () => {} } as unknown as EngineAdapter;
}

async function elapse(ms: number): Promise<void> {
  await vi.advanceTimersByTimeAsync(ms);
}

function committedFingerprint(): string {
  return stateFingerprint(useGameStore.getState().gameState!);
}

describe("staleStateWatchdog", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    useGameStore.getState().reset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  async function commitScreenState(state: GameState): Promise<void> {
    await processRemoteUpdate(snapshotOf(state), []);
    expect(committedFingerprint()).toBe(stateFingerprint(state));
  }

  it("heals a divergence after the arm delay", async () => {
    const screen = stateAt(1, 0);
    const ahead = stateAt(2, 1);
    await commitScreenState(screen);
    useGameStore.setState({ adapter: stubAdapter(() => snapshotOf(ahead)) });

    const watchdog = createStaleStateWatchdog();
    watchdog.start();
    try {
      await elapse(WATCHDOG_ARM_DELAY_MS - 1);
      expect(committedFingerprint()).toBe(stateFingerprint(screen));
      await elapse(1);
      expect(committedFingerprint()).toBe(stateFingerprint(ahead));
    } finally {
      watchdog.stop();
    }
  });

  it("a clean check disarms — no polling until the next commit re-arms", async () => {
    const screen = stateAt(1, 0);
    const laterDivergence = stateAt(2, 1);
    // The stub stamps a fresh seq per read (like the host's engine does) —
    // a pre-built snapshot would lose to the commit gate on later reads.
    let currentState = screen;
    await commitScreenState(screen);
    useGameStore.setState({ adapter: stubAdapter(() => snapshotOf(currentState)) });

    const watchdog = createStaleStateWatchdog();
    watchdog.start();
    try {
      // First check finds agreement and disarms.
      await elapse(WATCHDOG_ARM_DELAY_MS);
      expect(committedFingerprint()).toBe(stateFingerprint(screen));
      // The adapter now diverges WITHOUT any commit event. A poller would
      // pick this up; the causal design must stay asleep.
      currentState = laterDivergence;
      await elapse(WATCHDOG_ARM_DELAY_MS * 5);
      expect(committedFingerprint()).toBe(stateFingerprint(screen));
      // The next commit re-arms, and that check heals. Fresh object of
      // equal content: the engine always delivers fresh state objects, and
      // the store's change notification keys on the reference.
      await commitScreenState(stateAt(1, 0));
      await elapse(WATCHDOG_ARM_DELAY_MS);
      expect(committedFingerprint()).toBe(stateFingerprint(laterDivergence));
    } finally {
      watchdog.stop();
    }
  });

  it("each commit replaces the pending check instead of stacking", async () => {
    const screen = stateAt(1, 0);
    const ahead = stateAt(2, 1);
    await commitScreenState(screen);
    useGameStore.setState({ adapter: stubAdapter(() => snapshotOf(ahead)) });

    const watchdog = createStaleStateWatchdog();
    watchdog.start();
    try {
      // A commit half-way through the delay restarts the clock: the check
      // may only fire a full quiet delay after the LAST commit.
      await elapse(WATCHDOG_ARM_DELAY_MS / 2);
      await commitScreenState(stateAt(1, 0)); // fresh object, same content
      await elapse(WATCHDOG_ARM_DELAY_MS / 2);
      expect(committedFingerprint()).toBe(stateFingerprint(screen));
      await elapse(WATCHDOG_ARM_DELAY_MS / 2);
      expect(committedFingerprint()).toBe(stateFingerprint(ahead));
    } finally {
      watchdog.stop();
    }
  });

  it("resyncFromAdapter heals immediately and is a no-op when in agreement", async () => {
    const screen = stateAt(1, 0);
    const ahead = stateAt(2, 1);
    await commitScreenState(screen);

    let snapshot = snapshotOf(screen);
    useGameStore.setState({ adapter: stubAdapter(() => snapshot) });
    await resyncFromAdapter("test: agreement");
    expect(committedFingerprint()).toBe(stateFingerprint(screen));

    snapshot = snapshotOf(ahead);
    await resyncFromAdapter("test: divergence");
    expect(committedFingerprint()).toBe(stateFingerprint(ahead));
  });

  it("does nothing without an adapter or committed state", async () => {
    const watchdog = createStaleStateWatchdog();
    watchdog.start();
    try {
      await elapse(WATCHDOG_ARM_DELAY_MS * 2);
      expect(useGameStore.getState().gameState).toBeNull();
    } finally {
      watchdog.stop();
    }
    await resyncFromAdapter("test: empty store");
    expect(useGameStore.getState().gameState).toBeNull();
  });
});
