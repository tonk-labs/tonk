import { create } from "zustand";

interface TonkEvent {
  appName: string;
  timestamp: number;
  type: string;
}

interface EventTypeMap {
  [eventType: string]: TonkEvent[];
}

interface EventTimeMap {
  [timestamp: number]: TonkEvent;
}

interface EventState {
  _eventsByType: EventTypeMap;
  _eventsByTime: EventTimeMap;
  addEvent: (e: TonkEvent) => void;
  removeEvent: (e: TonkEvent) => void;
  getEventsByType: (type: string) => TonkEvent[];
}

export const useEventStore = create<EventState>((set, get) => ({
  _eventsByTime: {},
  _eventsByType: {},

  addEvent: (e: TonkEvent) => {
    set((state) => ({
      _eventsByType: {
        ...state._eventsByType,
        [e.type]: [...(state._eventsByType[e.type] || []), e],
      },
      _eventsByTime: {
        ...state._eventsByTime,
        [e.timestamp]: e,
      },
    }));
  },

  getEventsByType: (type: string) => {
    return get()._eventsByType[type] || [];
  },

  removeEvent: (e: TonkEvent) => {
    let modEventsByTime = get()._eventsByTime;
    delete modEventsByTime[e.timestamp];
    set((state) => ({
      _eventsByType: {
        ...state._eventsByType,
        [e.type]: [
          ...state._eventsByType[e.type].filter(
            (et) => et.timestamp === e.timestamp
          ),
        ],
      },
      _eventsByTime: {
        ...modEventsByTime,
      },
    }));
  },
}));
