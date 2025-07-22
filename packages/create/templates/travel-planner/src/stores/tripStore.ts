import { create } from "zustand";
import { sync, DocumentId } from "@tonk/keepsync";
import {
  Trip,
  Location,
  CalendarEvent,
  PlaceSuggestion,
  SharedNote,
  TripMember,
} from "../types/travel";

interface TripState {
  currentTrip: Trip | null;
  isLoading: boolean;

  // Trip management
  createTrip: (
    name: string,
    description: string,
    startDate: Date,
    endDate: Date,
    createdBy: string,
  ) => void;
  loadTrip: () => void;
  updateTrip: (updates: Partial<Trip>) => void;

  // Member management
  addMember: (member: Omit<TripMember, "joinedAt">) => void;
  removeMember: (memberId: string) => void;

  // Location management
  addLocation: (location: Omit<Location, "id" | "addedAt">) => void;
  removeLocation: (locationId: string) => void;
  updateLocation: (locationId: string, updates: Partial<Location>) => void;

  // Calendar event management
  addEvent: (event: Omit<CalendarEvent, "id" | "createdAt">) => void;
  removeEvent: (eventId: string) => void;
  updateEvent: (eventId: string, updates: Partial<CalendarEvent>) => void;

  // Place suggestions
  addSuggestion: (
    suggestion: Omit<
      PlaceSuggestion,
      "id" | "suggestedAt" | "votes" | "status"
    >,
  ) => void;
  voteSuggestion: (suggestionId: string, userId: string) => void;
  approveSuggestion: (suggestionId: string) => void;
  rejectSuggestion: (suggestionId: string) => void;

  // Shared notes
  addNote: (
    note: Omit<SharedNote, "id" | "createdAt" | "lastModifiedAt">,
  ) => void;
  updateNote: (noteId: string, updates: Partial<SharedNote>) => void;
  removeNote: (noteId: string) => void;
}

const generateId = () => Math.random().toString(36).substring(2, 11);

export const useTripStore = create<TripState>(
  sync(
    (set) => ({
      currentTrip: null,
      isLoading: true,

      loadTrip: () => {
        // The sync middleware should automatically load the persisted state
        // Set loading to false after a brief delay to allow sync to restore data
        setTimeout(() => {
          set({ isLoading: false });
        }, 2000);
      },

      createTrip: (name, description, startDate, endDate, createdBy) => {
        const newTrip: Trip = {
          id: generateId(),
          name,
          description,
          startDate: startDate.toISOString(),
          endDate: endDate.toISOString(),
          members: [
            {
              id: generateId(),
              name: createdBy,
              joinedAt: new Date().toISOString(),
            },
          ],
          locations: [],
          events: [],
          suggestions: [],
          notes: [],
          createdBy,
          createdAt: new Date().toISOString(),
        };
        set({ currentTrip: newTrip });
      },

      updateTrip: (updates) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? { ...state.currentTrip, ...updates }
            : null,
        }));
      },

      addMember: (member) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                members: [
                  ...state.currentTrip.members,
                  { ...member, joinedAt: new Date().toISOString() },
                ],
              }
            : null,
        }));
      },

      removeMember: (memberId) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                members: state.currentTrip.members.filter(
                  (m) => m.id !== memberId,
                ),
              }
            : null,
        }));
      },

      addLocation: (location) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                locations: [
                  ...state.currentTrip.locations,
                  {
                    ...location,
                    id: generateId(),
                    addedAt: new Date().toISOString(),
                  },
                ],
              }
            : null,
        }));
      },

      removeLocation: (locationId) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                locations: state.currentTrip.locations.filter(
                  (l) => l.id !== locationId,
                ),
              }
            : null,
        }));
      },

      updateLocation: (locationId, updates) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                locations: state.currentTrip.locations.map((l) =>
                  l.id === locationId ? { ...l, ...updates } : l,
                ),
              }
            : null,
        }));
      },

      addEvent: (event) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                events: [
                  ...state.currentTrip.events,
                  {
                    ...event,
                    id: generateId(),
                    createdAt: new Date().toISOString(),
                  },
                ],
              }
            : null,
        }));
      },

      removeEvent: (eventId) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                events: state.currentTrip.events.filter(
                  (e) => e.id !== eventId,
                ),
              }
            : null,
        }));
      },

      updateEvent: (eventId, updates) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                events: state.currentTrip.events.map((e) =>
                  e.id === eventId ? { ...e, ...updates } : e,
                ),
              }
            : null,
        }));
      },

      addSuggestion: (suggestion) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                suggestions: [
                  ...state.currentTrip.suggestions,
                  {
                    ...suggestion,
                    id: generateId(),
                    suggestedAt: new Date().toISOString(),
                    votes: [],
                    status: "pending" as const,
                  },
                ],
              }
            : null,
        }));
      },

      voteSuggestion: (suggestionId, userId) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                suggestions: state.currentTrip.suggestions.map((s) =>
                  s.id === suggestionId
                    ? {
                        ...s,
                        votes: s.votes.includes(userId)
                          ? s.votes.filter((id) => id !== userId)
                          : [...s.votes, userId],
                      }
                    : s,
                ),
              }
            : null,
        }));
      },

      approveSuggestion: (suggestionId) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                suggestions: state.currentTrip.suggestions.map((s) =>
                  s.id === suggestionId
                    ? { ...s, status: "approved" as const }
                    : s,
                ),
              }
            : null,
        }));
      },

      rejectSuggestion: (suggestionId) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                suggestions: state.currentTrip.suggestions.map((s) =>
                  s.id === suggestionId
                    ? { ...s, status: "rejected" as const }
                    : s,
                ),
              }
            : null,
        }));
      },

      addNote: (note) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                notes: [
                  ...state.currentTrip.notes,
                  {
                    ...note,
                    id: generateId(),
                    createdAt: new Date().toISOString(),
                    lastModifiedAt: new Date().toISOString(),
                  },
                ],
              }
            : null,
        }));
      },

      updateNote: (noteId, updates) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                notes: state.currentTrip.notes.map((n) =>
                  n.id === noteId
                    ? {
                        ...n,
                        ...updates,
                        lastModifiedAt: new Date().toISOString(),
                      }
                    : n,
                ),
              }
            : null,
        }));
      },

      removeNote: (noteId) => {
        set((state) => ({
          currentTrip: state.currentTrip
            ? {
                ...state.currentTrip,
                notes: state.currentTrip.notes.filter((n) => n.id !== noteId),
              }
            : null,
        }));
      },
    }),
    {
      docId: "travel-planner-trip" as DocumentId,
      initTimeout: 30000,
      onInitError: (error) =>
        console.error("Trip store sync initialization error:", error),
    },
  ),
);
