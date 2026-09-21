import { beforeEach, describe, expect, it, vi } from "vitest";
import { useNativeSteer } from "./useNativeSteer";
import { submitNativeSteer } from "@/lib/backend";

const harness = vi.hoisted(() => ({ slots: [] as unknown[], cursor: 0, turn: "turn-1" }));
vi.mock("react", () => ({
  useEffect: () => undefined,
  useRef: (initial: unknown) => {
    const index = harness.cursor++;
    harness.slots[index] ??= { current: initial };
    return harness.slots[index];
  },
  useState: (initial: unknown) => {
    const index = harness.cursor++;
    if (!(index in harness.slots)) harness.slots[index] = initial;
    return [
      harness.slots[index],
      (value: unknown) => {
        harness.slots[index] = value;
      },
    ];
  },
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@/stores/sessionStore", () => ({
  useSessionStore: (selector: (state: unknown) => unknown) =>
    selector({ liveBySession: { s: { input_queue_id: "runtime" } } }),
}));
vi.mock("@/stores/steerStore", () => ({
  useSteerStore: Object.assign(
    (selector: (state: unknown) => unknown) =>
      selector({ snapshots: { s: { instance_id: "runtime", turn_id: harness.turn } } }),
    { getState: () => ({ onSnapshot: vi.fn() }) },
  ),
}));
vi.mock("@/lib/backend", () => ({
  submitNativeSteer: vi.fn(),
  getNativeSteerSnapshot: vi.fn().mockResolvedValue({}),
}));

function useTestRender() {
  harness.cursor = 0;
  return useNativeSteer("s");
}
const receipt = (status: "accepted" | "applied" | "rejected" | "cancelled") => ({
  session_record_id: "s",
  instance_id: "runtime",
  turn_id: "turn-1",
  input_id: "i",
  text: "draft",
  image_count: 0,
  generation: 1,
  status,
  error: null,
});

describe("shared Composer/modal steer submission", () => {
  beforeEach(() => {
    harness.slots = [];
    harness.turn = "turn-1";
    vi.mocked(submitNativeSteer).mockReset();
  });
  it("retries transport failure with the same ID and old turn, preserving draft input", async () => {
    vi.mocked(submitNativeSteer)
      .mockRejectedValueOnce("transport lost")
      .mockResolvedValueOnce(receipt("applied"));
    expect(await useTestRender().submit("draft", ["image.png"])).toBe(false);
    const first = vi.mocked(submitNativeSteer).mock.calls[0];
    harness.turn = "turn-2";
    expect(await useTestRender().submit("draft", ["image.png"])).toBe(true);
    expect(vi.mocked(submitNativeSteer).mock.calls[1]).toEqual(first);
  });
  it("a definitive stale rejection lets unchanged draft target the next turn", async () => {
    vi.mocked(submitNativeSteer)
      .mockRejectedValueOnce({ kind: "rejected", message: "stale" })
      .mockResolvedValueOnce(receipt("accepted"));
    expect(await useTestRender().submit("draft", [])).toBe(false);
    const firstId = vi.mocked(submitNativeSteer).mock.calls[0][2];
    harness.turn = "turn-2";
    expect(await useTestRender().submit("draft", [])).toBe(true);
    expect(vi.mocked(submitNativeSteer).mock.calls[1][1]).toBe("turn-2");
    expect(vi.mocked(submitNativeSteer).mock.calls[1][2]).not.toBe(firstId);
  });
  it.each(["rejected", "cancelled"] as const)(
    "a replayed %s receipt is not a successful submission",
    async (status) => {
      vi.mocked(submitNativeSteer).mockResolvedValue(receipt(status));
      expect(await useTestRender().submit("draft", [])).toBe(false);
      expect(useTestRender().error).toBe(`steer.${status}`);
    },
  );
  it("rejects UTF-8 byte overflow without dispatching IPC", async () => {
    expect(await useTestRender().submit("中".repeat(70000), [])).toBe(false);
    expect(submitNativeSteer).not.toHaveBeenCalled();
  });
});
