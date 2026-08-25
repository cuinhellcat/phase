/**
 * Self-healing for a stale game screen — event-armed, no polling.
 *
 * The screen renders the last snapshot committed through the dispatch
 * pipeline, and every delivery path hands a fresh snapshot to that pipeline
 * exactly once. A delivery whose processing rejects is gone — nothing
 * retries it — so the screen keeps showing the previous state while the
 * engine (and every other client it serves) has moved on. Observed as a
 * pod-draft match host frozen on the "opponent is deciding their opening
 * hand" overlay after both players kept, while the guest played on (#7836).
 *
 * Recovery needs no wire traffic: the adapter already holds the newest
 * state this client is entitled to (the host's adapter asks its own engine;
 * a guest's adapter caches the last inbound state-bearing message).
 *
 * Causality, not a loop: every committed snapshot arms ONE deferred check;
 * the next commit replaces it. A check that finds screen and adapter in
 * agreement disarms — nothing runs again until the next commit. Only a
 * check that finds a persistent divergence re-commits the adapter snapshot
 * through the ordinary remote-update pipeline (dispatch mutex and the
 * store's commit gate still apply), and that commit arms the next check.
 * Steady state costs nothing; the arm delay doubles as transient-blip
 * protection (same reasoning as `STUCK_DEBOUNCE_MS`).
 */
import type { EngineSnapshot, GameState } from "../adapter/types";
import { debugLog } from "./debugLog";
import { isDispatchIdle, processRemoteUpdate } from "./dispatch";
import { useGameStore } from "../stores/gameStore";

/** Quiet time after a commit before its one-shot divergence check fires. */
export const WATCHDOG_ARM_DELAY_MS = 10_000;

/**
 * The slice of the state a viewer can see go stale. `waiting_for` carries the
 * pending-decision sets (the frozen overlay's data source); the rest pins the
 * game's coarse position so a stall outside any decision point still differs.
 */
export function stateFingerprint(state: GameState): string {
  return JSON.stringify({
    waiting_for: state.waiting_for,
    priority_player: state.priority_player,
    turn_number: state.turn_number,
    phase: state.phase,
    stack_len: state.stack.length,
  });
}

async function readAdapterSnapshot(): Promise<EngineSnapshot | null> {
  const { adapter } = useGameStore.getState();
  if (!adapter) return null;
  try {
    return await adapter.getSnapshot();
  } catch {
    // A guest without a cached state yet, or an adapter mid-teardown —
    // nothing to compare against, so nothing to heal.
    return null;
  }
}

/**
 * One comparison + (on divergence) one recommit. Exported for the
 * delivery-failure handlers: a caught rejection is positive knowledge that
 * exactly one update was lost, so they re-sync immediately, no arm delay.
 */
export async function resyncFromAdapter(reason: string): Promise<void> {
  const snapshot = await readAdapterSnapshot();
  if (!snapshot) return;
  const committed = useGameStore.getState().gameState;
  if (!committed) return;
  if (stateFingerprint(snapshot.state) === stateFingerprint(committed)) return;
  debugLog(`stale-screen resync (${reason})`, "warn");
  await processRemoteUpdate(snapshot, [], undefined);
}

export interface StaleStateWatchdog {
  start(): void;
  stop(): void;
}

export function createStaleStateWatchdog(): StaleStateWatchdog {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let unsubscribe: (() => void) | null = null;
  let checking = false;

  function disarm(): void {
    if (timer != null) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function arm(): void {
    disarm();
    timer = setTimeout(() => {
      timer = null;
      if (checking) return; // a still-running check re-arms on its own
      checking = true;
      void check().finally(() => {
        checking = false;
      });
    }, WATCHDOG_ARM_DELAY_MS);
  }

  async function check(): Promise<void> {
    // A busy pipeline will normally re-arm through its own commit, but a
    // queue that drains through rejections commits nothing — keep our own
    // re-arm so that case still gets its check.
    if (!isDispatchIdle()) {
      arm();
      return;
    }
    const adapterBefore = useGameStore.getState().adapter;
    const snapshot = await readAdapterSnapshot();
    const { adapter, gameState } = useGameStore.getState();
    // The await may cross a game teardown/swap — a snapshot from the old
    // adapter must not be compared to (or committed over) the new game.
    if (!snapshot || !gameState || adapter !== adapterBefore) return;
    if (!isDispatchIdle()) {
      arm();
      return;
    }
    if (stateFingerprint(snapshot.state) === stateFingerprint(gameState)) return;
    debugLog("stale-screen resync (deferred check: snapshot diverged)", "warn");
    await processRemoteUpdate(snapshot, [], undefined);
  }

  return {
    start(): void {
      if (unsubscribe) return;
      unsubscribe = useGameStore.subscribe(
        (s) => s.gameState,
        () => arm(),
      );
      // The commit that installed this game predates start() — give it its
      // one check too.
      arm();
    },
    stop(): void {
      disarm();
      if (unsubscribe) {
        unsubscribe();
        unsubscribe = null;
      }
    },
  };
}
