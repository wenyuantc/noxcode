import { beforeEach, describe, expect, it } from "vitest";
import { useSteerStore } from "./steerStore";
import type { AgentSessionStarted, NativeSteerSnapshot } from "@/lib/types";

const snapshot = (revision: number, turn = "turn", instance = "runtime"): NativeSteerSnapshot => ({
  session_record_id: "s",
  instance_id: instance,
  turn_id: turn,
  revision,
  receipts: [],
});
const started = (instance = "runtime"): AgentSessionStarted => ({
  session_record_id: "s",
  input_queue_id: instance,
  profile_id: "",
  workspace_id: "w",
  session_kind: "execution",
});

describe("steer runtime events", () => {
  beforeEach(() => useSteerStore.setState(useSteerStore.getInitialState(), true));
  it("preserves a newer turn event that arrives before a metadata-free start response", () => {
    useSteerStore.getState().onSnapshot(snapshot(3));
    useSteerStore.getState().onStarted(started());
    expect(useSteerStore.getState().snapshots.s.turn_id).toBe("turn");
    useSteerStore.getState().onSnapshot(snapshot(2, "old-turn"));
    expect(useSteerStore.getState().snapshots.s.turn_id).toBe("turn");
  });
  it("clears old state on restart and rejects an event from the prior runtime", () => {
    useSteerStore.getState().onStarted(started());
    useSteerStore.getState().onSnapshot(snapshot(3));
    useSteerStore.getState().onStarted(started("new-runtime"));
    expect(useSteerStore.getState().snapshots.s).toBeUndefined();
    useSteerStore.getState().onSnapshot(snapshot(99));
    expect(useSteerStore.getState().snapshots.s).toBeUndefined();
    useSteerStore.getState().onSnapshot(snapshot(1, "new-turn", "new-runtime"));
    expect(useSteerStore.getState().snapshots.s.turn_id).toBe("new-turn");
  });
  it("exit cancels pending receipts without allowing late events to reopen the turn", () => {
    const value = snapshot(2);
    value.receipts = [
      {
        session_record_id: "s",
        instance_id: "runtime",
        turn_id: "turn",
        input_id: "input",
        text: "restore me",
        image_count: 1,
        generation: 1,
        status: "accepted",
        error: null,
      },
    ];
    useSteerStore.getState().onSnapshot(value);
    useSteerStore.getState().onExit("s", "runtime");
    useSteerStore.getState().onSnapshot({ ...value, revision: 10 });
    const ended = useSteerStore.getState().snapshots.s;
    expect(ended.turn_id).toBeNull();
    expect(ended.receipts[0]).toMatchObject({
      status: "cancelled",
      text: "restore me",
      image_count: 1,
    });
  });
  it("rejects a retired start after a newer runtime is active", () => {
    const state = useSteerStore.getState();
    state.onStarted(started("old"));
    state.onSnapshot(snapshot(1, "old-turn", "old"));
    state.onExit("s", "old");
    state.onStarted(started("new"));
    state.onSnapshot(snapshot(1, "new-turn", "new"));
    state.onStarted(started("old"));
    expect(useSteerStore.getState().instances.s).toBe("new");
    expect(useSteerStore.getState().snapshots.s.turn_id).toBe("new-turn");
  });
  it("does not revive an exited runtime through duplicate start and equal-revision events", () => {
    const state = useSteerStore.getState();
    state.onStarted(started());
    const previous = snapshot(3);
    state.onSnapshot(previous);
    state.onExit("s", "runtime");
    state.onStarted(started());
    state.onSnapshot(previous);
    expect(useSteerStore.getState().ended.s).toBe(true);
    expect(useSteerStore.getState().snapshots.s.turn_id).toBeNull();
  });
  it.each(["snapshot-first", "event-first"])(
    "opens steering with %s without losing receipt updates",
    (ordering) => {
      const state = useSteerStore.getState();
      state.onStarted(started());
      state.onSnapshot({ ...snapshot(8, "turn-1"), turn_id: null });
      const working = {
        session_record_id: "s",
        instance_id: "runtime",
        turn_id: "turn-2",
        revision: 10,
        state: "working",
        steer_turn_id: "turn-2",
      };
      const opening = {
        ...snapshot(9, "turn-2"),
        lifecycle: { ...working, revision: 9 },
        receipts: [
          {
            session_record_id: "s",
            instance_id: "runtime",
            turn_id: "turn-1",
            input_id: "prior",
            text: "prior receipt",
            image_count: 0,
            generation: 1,
            status: "applied" as const,
            error: null,
          },
        ],
      };
      if (ordering === "event-first") {
        state.onTurnState(working);
        expect(useSteerStore.getState().snapshots.s.turn_id).toBe("turn-2");
        state.onSnapshot(opening);
      } else {
        state.onSnapshot(opening);
        state.onTurnState(working);
      }
      expect(useSteerStore.getState().snapshots.s.turn_id).toBe("turn-2");
      expect(useSteerStore.getState().snapshots.s.receipts[0].status).toBe("applied");
      expect(useSteerStore.getState().lifecycles.s.revision).toBe(10);
    },
  );

  it.each(["snapshot-first", "event-first"])(
    "seals steering with %s and never reopens from an older active snapshot",
    (ordering) => {
      const state = useSteerStore.getState();
      state.onStarted(started());
      const working = {
        session_record_id: "s",
        instance_id: "runtime",
        turn_id: "turn-2",
        revision: 10,
        state: "working",
        steer_turn_id: "turn-2",
      };
      const active = { ...snapshot(11, "turn-2"), lifecycle: working };
      state.onSnapshot(active);
      const closed = { ...snapshot(12, "turn-2"), turn_id: null, lifecycle: working };
      const waiting = { ...working, revision: 13, state: "waiting_input", steer_turn_id: null };
      if (ordering === "event-first") {
        state.onTurnState(waiting);
        expect(useSteerStore.getState().snapshots.s.turn_id).toBeNull();
        state.onSnapshot(closed);
      } else {
        state.onSnapshot(closed);
        state.onTurnState(waiting);
      }
      state.onSnapshot(active);
      expect(useSteerStore.getState().snapshots.s.turn_id).toBeNull();
      state.onTurnState({ ...waiting, state: "working", revision: 14 });
      state.onSnapshot(active);
      expect(useSteerStore.getState().snapshots.s.turn_id).toBeNull();
      state.onTurnState({ ...working, turn_id: "turn-3", steer_turn_id: "turn-3", revision: 16 });
      state.onSnapshot({ ...closed, revision: 15 });
      expect(useSteerStore.getState().snapshots.s.turn_id).toBe("turn-3");
    },
  );
  it("keeps receipt status monotonic when receipt snapshots trail newer lifecycle events", () => {
    const state = useSteerStore.getState();
    state.onStarted(started());
    const receipt = {
      session_record_id: "s",
      instance_id: "runtime",
      turn_id: "turn-1",
      input_id: "receipt",
      text: "original",
      image_count: 0,
      generation: 1,
      status: "accepted" as const,
      error: null,
    };
    const accepted = { ...snapshot(3, "turn-1"), receipts: [receipt] };
    state.onSnapshot(accepted);
    state.onTurnState({
      session_record_id: "s",
      instance_id: "runtime",
      turn_id: "turn-2",
      revision: 7,
      state: "working",
      steer_turn_id: "turn-2",
    });
    state.onSnapshot({ ...snapshot(4, "turn-1"), receipts: [{ ...receipt, status: "applied" }] });
    state.onSnapshot(accepted);
    state.onSnapshot({
      ...snapshot(5, "turn-1"),
      turn_id: null,
      receipts: [{ ...receipt, status: "applied" }],
    });
    expect(useSteerStore.getState().snapshots.s.turn_id).toBe("turn-2");
    expect(useSteerStore.getState().snapshots.s.receipts[0].status).toBe("applied");
    state.onTurnState({
      session_record_id: "s",
      instance_id: "runtime",
      turn_id: "turn-2",
      revision: 8,
      state: "waiting_input",
      steer_turn_id: null,
    });
    state.onSnapshot(snapshot(6, "turn-2"));
    expect(useSteerStore.getState().snapshots.s.turn_id).toBeNull();
    expect(useSteerStore.getState().snapshots.s.receipts[0].status).toBe("applied");
  });

  it("uses explicit closed admission on a working idle-compaction event", () => {
    const state = useSteerStore.getState();
    state.onStarted(started());
    state.onSnapshot(snapshot(2, "completed"));
    state.onTurnState({
      session_record_id: "s",
      instance_id: "runtime",
      turn_id: "completed",
      revision: 5,
      state: "working",
      steer_turn_id: null,
    });
    expect(useSteerStore.getState().snapshots.s.turn_id).toBeNull();
    expect(useSteerStore.getState().lifecycles.s.state).toBe("working");
    state.onSnapshot(snapshot(3, "completed"));
    expect(useSteerStore.getState().snapshots.s.turn_id).toBeNull();
  });
});
