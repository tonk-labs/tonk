import React, { useState } from "react";
import { useNotesStore, Note } from "../../stores/NotesStore";
import { Save, Plus, Trash2 } from "lucide-react";

/**
 * Component props for the NotesApp component
 */
export interface NotesAppProps {
  /**
   * Optional title for the notes app
   */
  title?: string;
}

/**
 * A simple notes application that allows creating, editing and deleting notes
 * using keepsync for real-time synchronization across clients
 */
const NotesApp: React.FC<NotesAppProps> = ({ title = "My Notes" }) => {
  const { notes, addNote, updateNote, deleteNote } = useNotesStore();
  const [selectedNoteId, setSelectedNoteId] = useState<string | null>(null);
  const [currentContent, setCurrentContent] = useState<string>("");
  const [isEditing, setIsEditing] = useState<boolean>(false);

  // Get the selected note
  const selectedNote = selectedNoteId 
    ? notes.find(note => note.id === selectedNoteId) 
    : null;

  // Create a new note
  const handleCreateNote = () => {
    const newNoteContent = "New note";
    const id = crypto.randomUUID();
    const now = Date.now();
    
    // Create the note object with the same logic as in the store
    const newNote = {
      id,
      content: newNoteContent,
      createdAt: now,
      updatedAt: now
    };
    
    // Add the note to the store with our generated ID
    addNote(newNoteContent, id);
    
    // Immediately select the new note without waiting for state updates
    setSelectedNoteId(id);
    setCurrentContent(newNoteContent);
    setIsEditing(true);
  };

  // Select a note for viewing/editing
  const handleSelectNote = (note: Note) => {
    setSelectedNoteId(note.id);
    setCurrentContent(note.content);
    setIsEditing(false);
  };

  // Update the selected note
  const handleSaveNote = () => {
    if (selectedNoteId && currentContent.trim()) {
      updateNote(selectedNoteId, currentContent);
      setIsEditing(false);
    }
  };

  // Delete the selected note
  const handleDeleteNote = () => {
    if (selectedNoteId) {
      deleteNote(selectedNoteId);
      setSelectedNoteId(null);
      setCurrentContent("");
      setIsEditing(false);
    }
  };

  // Format the date to a readable string
  const formatDate = (timestamp: number) => {
    return new Date(timestamp).toLocaleString();
  };

  return (
    <div className="flex h-full rounded-lg border border-gray-200 overflow-hidden">
      {/* Notes sidebar */}
      <div className="w-1/3 border-r border-gray-200 bg-gray-50 flex flex-col">
        <div className="p-4 border-b border-gray-200 bg-white flex justify-between items-center">
          <h2 className="text-lg font-semibold">{title}</h2>
          <button 
            onClick={handleCreateNote}
            className="p-2 rounded-full hover:bg-gray-100"
            aria-label="Create new note"
          >
            <Plus size={20} />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto">
          {notes.length === 0 ? (
            <div className="p-4 text-gray-500 text-center">
              No notes yet. Click the + button to create one.
            </div>
          ) : (
            <ul>
              {notes.map((note) => (
                <li key={note.id}>
                  <button
                    onClick={() => handleSelectNote(note)}
                    className={`w-full text-left p-4 border-b border-gray-200 hover:bg-gray-100 transition-colors ${
                      selectedNoteId === note.id ? "bg-blue-50" : ""
                    }`}
                  >
                    <div className="font-medium line-clamp-2">
                      {note.content.split("\n")[0] || "Untitled"}
                    </div>
                    <div className="text-xs text-gray-500 mt-1">
                      {formatDate(note.updatedAt)}
                    </div>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {/* Note editor */}
      <div className="w-2/3 flex flex-col">
        {selectedNote ? (
          <>
            <div className="p-4 border-b border-gray-200 flex justify-between items-center">
              <div>
                <div className="text-sm text-gray-500">
                  Last updated: {formatDate(selectedNote.updatedAt)}
                </div>
              </div>
              <div className="flex gap-2">
                <button
                  onClick={handleSaveNote}
                  disabled={!isEditing}
                  className={`p-2 rounded-full ${
                    isEditing
                      ? "text-blue-600 hover:bg-blue-50"
                      : "text-gray-400"
                  }`}
                  aria-label="Save note"
                >
                  <Save size={20} />
                </button>
                <button
                  onClick={handleDeleteNote}
                  className="p-2 rounded-full text-red-600 hover:bg-red-50"
                  aria-label="Delete note"
                >
                  <Trash2 size={20} />
                </button>
              </div>
            </div>
            <div className="flex-1 p-4">
              <textarea
                value={currentContent}
                onChange={(e) => {
                  setCurrentContent(e.target.value);
                  setIsEditing(true);
                }}
                className="w-full h-full p-2 border border-gray-200 rounded-md resize-none focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                placeholder="Write your note here..."
              />
            </div>
          </>
        ) : (
          <div className="flex-1 flex items-center justify-center text-gray-500">
            Select a note or create a new one to get started.
          </div>
        )}
      </div>
    </div>
  );
};

export default NotesApp;