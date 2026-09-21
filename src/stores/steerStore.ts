import { create } from "zustand";
import type { AgentSessionStarted, NativeSteerSnapshot, NativeTurnState } from "@/lib/types";

interface SteerAdmission {
  instance_id: string;
  turn_id: string | null;
  revision: number;
}

interface SteerState {
  snapshots: Record<string, NativeSteerSnapshot>;
  instances: Record<string, string>;
  ended: Record<string, boolean>;
  retired: Record<string, Record<string, true>>;
  lifecycles: Record<string, NativeTurnState>;
  handledTurnRevisions: Record<string, number>;
  admissions: Record<string, SteerAdmission>;
  onSnapshot: (snapshot: NativeSteerSnapshot) => boolean;
  onStarted: (session: AgentSessionStarted) => boolean;
  onTurnState: (event: NativeTurnState) => boolean;
  onExit: (sessionId: string, instanceId: string) => boolean;
  acceptsRuntime: (sessionId: string, instanceId?: string | null) => boolean;
  isCurrentTurn: (event: NativeTurnState) => boolean;
  isCurrentExit: (sessionId: string, instanceId: string) => boolean;
}

export const useSteerStore = create<SteerState>((set, get) => ({
  snapshots: {},
  instances: {},
  ended: {},
  retired: {},
  lifecycles: {},
  handledTurnRevisions: {},
  admissions: {},
  onSnapshot: (snapshot) => {
    const state = get();
    const id = snapshot.session_record_id;
    const instance = state.instances[id] ?? state.lifecycles[id]?.instance_id;
    const previous = state.snapshots[id];
    if (instance && instance !== snapshot.instance_id) return false;
    if ((state.ended[id] || state.retired[id]?.[snapshot.instance_id]) && snapshot.turn_id)
      return false;
    if (
      previous?.instance_id === snapshot.instance_id &&
      previous.revision > snapshot.revision &&
      !state.ended[id]
    )
      return false;
    const ids = new Set(snapshot.receipts.map((r) => r.input_id));
    const history = (previous?.receipts ?? []).filter((r) => !ids.has(r.input_id));
    const observed =
      snapshot.lifecycle ??
      (snapshot.turn_id
        ? {
            session_record_id: id,
            instance_id: snapshot.instance_id,
            turn_id: snapshot.turn_id,
            state: "working",
            revision: snapshot.revision,
          }
        : undefined);
    const lifecycle =
      !state.ended[id] &&
      observed &&
      (!state.lifecycles[id] || observed.revision >= state.lifecycles[id].revision)
        ? observed
        : state.lifecycles[id];
    // Snapshot revision orders receipt data; admission revision independently
    // orders opening/sealing facts supplied by snapshots and lifecycle events.
    const admission =
      !state.admissions[id] || snapshot.revision >= state.admissions[id].revision
        ? {
            instance_id: snapshot.instance_id,
            turn_id: snapshot.turn_id,
            revision: snapshot.revision,
          }
        : state.admissions[id];
    set({
      lifecycles: lifecycle ? { ...state.lifecycles, [id]: lifecycle } : state.lifecycles,
      admissions: { ...state.admissions, [id]: admission },
      snapshots: {
        ...state.snapshots,
        [id]: {
          ...snapshot,
          turn_id: admission.turn_id,
          lifecycle,
          receipts: [...history, ...snapshot.receipts].slice(-256),
        },
      },
    });
    return true;
  },
  onStarted: (session) => {
    const state = get();
    const id = session.session_record_id;
    const instance = session.input_queue_id;
    if (!instance) return !state.instances[id];
    if (state.retired[id]?.[instance] || (state.ended[id] && state.instances[id] === instance))
      return false;
    const previous = state.instances[id] ?? state.snapshots[id]?.instance_id;
    const snapshots = { ...state.snapshots };
    const lifecycles = { ...state.lifecycles };
    const handledTurnRevisions = { ...state.handledTurnRevisions };
    const admissions = { ...state.admissions };
    if (admissions[id]?.instance_id !== instance) delete admissions[id];
    // A same-instance turn event may precede both start notifications.
    if (snapshots[id]?.instance_id !== instance) delete snapshots[id];
    if (lifecycles[id]?.instance_id !== instance) {
      delete lifecycles[id];
      delete handledTurnRevisions[id];
    }
    set({
      snapshots,
      lifecycles,
      handledTurnRevisions,
      admissions,
      instances: { ...state.instances, [id]: instance },
      ended: { ...state.ended, [id]: false },
      retired:
        previous && previous !== instance
          ? { ...state.retired, [id]: { ...state.retired[id], [previous]: true } }
          : state.retired,
    });
    return true;
  },
  acceptsRuntime: (id, instance) => {
    const state = get();
    const known =
      state.instances[id] ?? state.snapshots[id]?.instance_id ?? state.lifecycles[id]?.instance_id;
    if (!instance) return !known;
    return (
      !state.retired[id]?.[instance] &&
      !(state.ended[id] && known === instance) &&
      (!known || known === instance)
    );
  },
  onTurnState: (event) => {
    const state = get();
    const id = event.session_record_id;
    if (
      !event.instance_id ||
      !event.turn_id ||
      !Number.isSafeInteger(event.revision) ||
      !state.acceptsRuntime(id, event.instance_id)
    )
      return false;
    const previous = state.lifecycles[id];
    if (
      previous &&
      (event.revision < previous.revision ||
        (event.revision === previous.revision && event.turn_id !== previous.turn_id))
    )
      return false;
    if ((state.handledTurnRevisions[id] ?? -1) >= event.revision) return false;
    const suppliedAdmission = event.steer_turn_id !== undefined || event.state === "waiting_input";
    const admission =
      suppliedAdmission &&
      (!state.admissions[id] || event.revision >= state.admissions[id].revision)
        ? {
            instance_id: event.instance_id,
            turn_id: event.steer_turn_id ?? null,
            revision: event.revision,
          }
        : state.admissions[id];
    const snapshot = state.snapshots[id];
    set({
      instances: { ...state.instances, [id]: event.instance_id },
      lifecycles: { ...state.lifecycles, [id]: event },
      handledTurnRevisions: { ...state.handledTurnRevisions, [id]: event.revision },
      admissions: admission ? { ...state.admissions, [id]: admission } : state.admissions,
      // Do not advance receipt revision: an earlier same-turn snapshot may still
      // contain newer receipt statuses than the last snapshot we actually saw.
      snapshots: admission
        ? {
            ...state.snapshots,
            [id]: {
              session_record_id: id,
              instance_id: event.instance_id,
              revision: snapshot?.revision ?? 0,
              receipts: snapshot?.receipts ?? [],
              turn_id: admission.turn_id,
              lifecycle: event,
            },
          }
        : state.snapshots,
    });
    return true;
  },
  isCurrentTurn: (event) => {
    const state = get();
    const current = state.lifecycles[event.session_record_id];
    return (
      state.acceptsRuntime(event.session_record_id, event.instance_id) &&
      current?.turn_id === event.turn_id &&
      current.revision === event.revision &&
      current.state === event.state
    );
  },
  isCurrentExit: (id, instance) => {
    const state = get();
    return Boolean(
      state.ended[id] &&
      (state.instances[id] ?? state.snapshots[id]?.instance_id ?? "") === instance,
    );
  },
  onExit: (id, instance) => {
    const state = get();
    const previous = state.snapshots[id];
    const current =
      state.instances[id] ?? previous?.instance_id ?? state.lifecycles[id]?.instance_id;
    // An exit can retire only its named runtime, including an old runtime whose
    // start was missed. It cannot remove or retire the current live instance.
    if (current && current !== instance) {
      if (instance)
        set({ retired: { ...state.retired, [id]: { ...state.retired[id], [instance]: true } } });
      return false;
    }
    if (state.ended[id]) return false;
    set({
      instances: instance ? { ...state.instances, [id]: instance } : state.instances,
      retired: instance
        ? { ...state.retired, [id]: { ...state.retired[id], [instance]: true } }
        : state.retired,
      ended: { ...state.ended, [id]: true },
      admissions: state.admissions[id]
        ? { ...state.admissions, [id]: { ...state.admissions[id], turn_id: null } }
        : state.admissions,
      snapshots: previous
        ? {
            ...state.snapshots,
            [id]: {
              ...previous,
              turn_id: null,
              receipts: previous.receipts.map((r) =>
                r.status === "accepted" ? { ...r, status: "cancelled" as const } : r,
              ),
            },
          }
        : state.snapshots,
    });
    return true;
  },
}));
