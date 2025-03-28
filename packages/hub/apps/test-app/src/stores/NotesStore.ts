import { sync } from "@tonk/keepsync";
import { create } from "zustand";


// Define the Note type
export interface Note {
  id: string;
  content: string;
  createdAt: number;
  updatedAt: number;
}

// Define the data structure
interface NotesData {
  notes: Note[];
}

// Define the store state
interface NotesState extends NotesData {
  addNote: (content: string, id?: string) => void;
  updateNote: (id: string, content: string) => void;
  deleteNote: (id: string) => void;
}

// Create a synced store for notes
export const useNotesStore = create<NotesState>(
  sync(
    (set) => ({
      notes: [],

      // Add a new note
      addNote: (content: string, id?: string) => {
        const now = Date.now();
        set((state) => ({
          notes: [
            ...state.notes,
            {
              id: id || crypto.randomUUID(),
              content,
              createdAt: now,
              updatedAt: now,
            },
          ],
        }));
      },

      // Update a note's content
      updateNote: (id: string, content: string) => {
        set((state) => ({
          notes: state.notes.map((note) =>
            note.id === id 
              ? { ...note, content, updatedAt: Date.now() } 
              : note
          ),
        }));
      },

      // Delete a note
      deleteNote: (id: string) => {
        set((state) => ({
          notes: state.notes.filter((note) => note.id !== id),
        }));
      },
    }),
    {
      // Unique document ID for this store
      docId: "notes-app",
    },
  ),
);