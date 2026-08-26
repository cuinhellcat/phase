import { beforeEach, expect, it, vi } from "vitest";

const { checkMock, isDesktopTauriMock } = vi.hoisted(() => ({
  checkMock: vi.fn(),
  isDesktopTauriMock: vi.fn(),
}));

vi.mock("../../services/platform", () => ({ isDesktopTauri: isDesktopTauriMock }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: checkMock }));

beforeEach(() => {
  vi.clearAllMocks();
  vi.resetModules();
});

it("does not import or invoke the updater on Android/iOS", async () => {
  isDesktopTauriMock.mockReturnValue(false);
  const updater = await import("../tauriUpdater");
  updater.registerTauriUpdater();
  expect(updater.checkForTauriUpdate()).toBe(false);
  expect(checkMock).not.toHaveBeenCalled();
});

it("retains desktop updater reachability", async () => {
  isDesktopTauriMock.mockReturnValue(true);
  checkMock.mockResolvedValue(null);
  const updater = await import("../tauriUpdater");
  updater.registerTauriUpdater();
  await vi.waitFor(() => expect(checkMock).toHaveBeenCalledOnce());
});
