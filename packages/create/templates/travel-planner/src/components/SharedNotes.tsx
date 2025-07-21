import React, { useState } from "react";
import { useTripStore } from "../stores/tripStore";
import { SharedNote } from "../types/travel";
import { Plus, Edit, Trash2, Save, X, Tag, User, Clock } from "lucide-react";

interface SharedNotesProps {
  currentUser: string;
}

export function SharedNotes({ currentUser }: SharedNotesProps) {
  const { currentTrip, addNote, updateNote, removeNote } = useTripStore();
  const [showNoteForm, setShowNoteForm] = useState(false);
  const [editingNote, setEditingNote] = useState<SharedNote | null>(null);
  const [noteForm, setNoteForm] = useState({
    title: "",
    content: "",
    tags: "",
  });
  const [searchTerm, setSearchTerm] = useState("");
  const [selectedTag, setSelectedTag] = useState("");

  if (!currentTrip) return <div className="card">No trip selected</div>;

  const notes = currentTrip.notes || [];
  const members = currentTrip.members || [];

  // Get all unique tags
  const allTags = Array.from(new Set(notes.flatMap((note) => note.tags)));

  // Filter notes based on search and tag
  const filteredNotes = notes.filter((note) => {
    const matchesSearch =
      searchTerm === "" ||
      note.title.toLowerCase().includes(searchTerm.toLowerCase()) ||
      note.content.toLowerCase().includes(searchTerm.toLowerCase());

    const matchesTag = selectedTag === "" || note.tags.includes(selectedTag);

    return matchesSearch && matchesTag;
  });

  const handleSubmitNote = (e: React.FormEvent) => {
    e.preventDefault();

    const tags = noteForm.tags
      .split(",")
      .map((tag) => tag.trim())
      .filter((tag) => tag.length > 0);

    if (editingNote) {
      updateNote(editingNote.id, {
        title: noteForm.title,
        content: noteForm.content,
        tags,
        lastModifiedBy: currentUser,
      });
    } else {
      addNote({
        title: noteForm.title,
        content: noteForm.content,
        tags,
        createdBy: currentUser,
        lastModifiedBy: currentUser,
      });
    }

    setShowNoteForm(false);
    setEditingNote(null);
    setNoteForm({
      title: "",
      content: "",
      tags: "",
    });
  };

  const handleEditNote = (note: SharedNote) => {
    setEditingNote(note);
    setNoteForm({
      title: note.title,
      content: note.content,
      tags: note.tags.join(", "),
    });
    setShowNoteForm(true);
  };

  const getMemberName = (memberId: string) => {
    const member = members.find(
      (m) => m.id === memberId || m.name === memberId,
    );
    return member?.name || memberId;
  };

  const formatDate = (date: Date) => {
    return new Date(date).toLocaleDateString("en-US", {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  };

  return (
    <div className="card">
      <div
        className="list-item"
        style={{
          padding: "1rem 0",
          background: "transparent",
        }}
      >
        <h2>Shared Notes</h2>
        <button
          onClick={() => setShowNoteForm(true)}
          className="btn btn-primary"
          style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}
        >
          <Plus size={20} />
          Add Note
        </button>
      </div>

      {/* Search and Filter */}
      <div style={{ marginBottom: "1.5rem" }}>
        <div style={{ display: "flex", gap: "1rem", marginBottom: "1rem" }}>
          <div style={{ flex: 1 }}>
            <input
              type="text"
              placeholder="Search notes..."
              value={searchTerm}
              onChange={(e) => setSearchTerm(e.target.value)}
              className="form-input"
            />
          </div>
          <div style={{ width: "12rem" }}>
            <select
              value={selectedTag}
              onChange={(e) => setSelectedTag(e.target.value)}
              className="form-select"
            >
              <option value="">All tags</option>
              {allTags.map((tag) => (
                <option key={tag} value={tag}>
                  {tag}
                </option>
              ))}
            </select>
          </div>
        </div>

        {allTags.length > 0 && (
          <div
            style={{
              display: "flex",
              flexWrap: "wrap",
              gap: "0.5rem",
              alignItems: "center",
            }}
          >
            <span style={{ fontSize: "0.875rem", opacity: 0.7 }}>Tags:</span>
            {allTags.map((tag) => (
              <button
                key={tag}
                onClick={() => setSelectedTag(selectedTag === tag ? "" : tag)}
                className={
                  selectedTag === tag ? "btn-secondary active" : "btn-secondary"
                }
                style={{ padding: "0.25rem 0.5rem", fontSize: "0.75rem" }}
              >
                {tag}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Note Form Modal */}
      {showNoteForm && (
        <div className="modal-overlay">
          <div
            className="modal-content"
            style={{ maxWidth: "48rem", maxHeight: "90vh", overflowY: "auto" }}
          >
            <h3 className="modal-title">
              {editingNote ? "Edit Note" : "Add New Note"}
            </h3>

            <form onSubmit={handleSubmitNote}>
              <div className="form-group">
                <label className="form-label">Title</label>
                <input
                  type="text"
                  value={noteForm.title}
                  onChange={(e) =>
                    setNoteForm({ ...noteForm, title: e.target.value })
                  }
                  className="form-input"
                  placeholder="Enter note title..."
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">Content</label>
                <textarea
                  value={noteForm.content}
                  onChange={(e) =>
                    setNoteForm({ ...noteForm, content: e.target.value })
                  }
                  className="form-textarea"
                  rows={8}
                  placeholder="Write your note here..."
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">Tags (comma-separated)</label>
                <input
                  type="text"
                  value={noteForm.tags}
                  onChange={(e) =>
                    setNoteForm({ ...noteForm, tags: e.target.value })
                  }
                  className="form-input"
                  placeholder="e.g., packing, itinerary, budget"
                />
              </div>

              <div className="modal-actions">
                <button
                  type="button"
                  onClick={() => {
                    setShowNoteForm(false);
                    setEditingNote(null);
                    setNoteForm({
                      title: "",
                      content: "",
                      tags: "",
                    });
                  }}
                  className="btn btn-secondary"
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "0.5rem",
                  }}
                >
                  <X size={16} />
                  Cancel
                </button>
                <button
                  type="submit"
                  className="btn btn-primary"
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "0.5rem",
                  }}
                >
                  <Save size={16} />
                  {editingNote ? "Update Note" : "Save Note"}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Notes List */}
      <div>
        {filteredNotes.length === 0 ? (
          <div className="empty-state">
            <div className="empty-state-icon">📝</div>
            {searchTerm || selectedTag ? (
              <h3>No notes match your search criteria</h3>
            ) : (
              <>
                <h3>No notes yet</h3>
                <p>Start by adding your first shared note!</p>
              </>
            )}
          </div>
        ) : (
          filteredNotes
            .sort(
              (a, b) =>
                new Date(b.lastModifiedAt).getTime() -
                new Date(a.lastModifiedAt).getTime(),
            )
            .map((note) => (
              <div
                key={note.id}
                className="card"
                style={{ background: "rgba(0, 0, 0, 0.02)" }}
              >
                <div
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "flex-start",
                    marginBottom: "1rem",
                  }}
                >
                  <div style={{ flex: 1 }}>
                    <h3
                      style={{
                        fontWeight: "600",
                        fontSize: "1.125rem",
                        marginBottom: "0.5rem",
                      }}
                    >
                      {note.title}
                    </h3>
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "1rem",
                        fontSize: "0.875rem",
                        opacity: 0.7,
                        marginBottom: "0.5rem",
                      }}
                    >
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: "0.25rem",
                        }}
                      >
                        <User size={14} />
                        <span>Created by {getMemberName(note.createdBy)}</span>
                      </div>
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: "0.25rem",
                        }}
                      >
                        <Clock size={14} />
                        <span>Modified {formatDate(note.lastModifiedAt)}</span>
                      </div>
                      {note.createdBy !== note.lastModifiedBy && (
                        <span style={{ fontSize: "0.75rem" }}>
                          (by {getMemberName(note.lastModifiedBy)})
                        </span>
                      )}
                    </div>
                  </div>
                  <div style={{ display: "flex", gap: "0.5rem" }}>
                    <button
                      onClick={() => handleEditNote(note)}
                      className="icon-btn"
                      style={{ color: "#2c2c2c" }}
                      title="Edit note"
                    >
                      <Edit size={16} />
                    </button>
                    <button
                      onClick={() => removeNote(note.id)}
                      className="icon-btn"
                      style={{ color: "#dc2626" }}
                      title="Delete note"
                    >
                      <Trash2 size={16} />
                    </button>
                  </div>
                </div>

                <div style={{ marginBottom: "1rem" }}>
                  <div
                    style={{
                      whiteSpace: "pre-wrap",
                      opacity: 0.8,
                      lineHeight: 1.6,
                    }}
                  >
                    {note.content}
                  </div>
                </div>

                {note.tags.length > 0 && (
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "0.5rem",
                      flexWrap: "wrap",
                    }}
                  >
                    <Tag size={14} style={{ opacity: 0.5 }} />
                    {note.tags.map((tag) => (
                      <span
                        key={tag}
                        onClick={() => setSelectedTag(tag)}
                        style={{
                          padding: "0.25rem 0.5rem",
                          background: "rgba(44, 44, 44, 0.1)",
                          color: "#2c2c2c",
                          borderRadius: "20px",
                          fontSize: "0.75rem",
                          cursor: "pointer",
                          transition: "all 0.3s ease",
                        }}
                        onMouseEnter={(e) => {
                          e.currentTarget.style.background =
                            "rgba(44, 44, 44, 0.15)";
                        }}
                        onMouseLeave={(e) => {
                          e.currentTarget.style.background =
                            "rgba(44, 44, 44, 0.1)";
                        }}
                      >
                        {tag}
                      </span>
                    ))}
                  </div>
                )}
              </div>
            ))
        )}
      </div>

      {/* Notes Summary */}
      {notes.length > 0 && (
        <div
          style={{
            marginTop: "1.5rem",
            paddingTop: "1rem",
            borderTop: "1px solid rgba(0, 0, 0, 0.1)",
          }}
        >
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              fontSize: "0.875rem",
              opacity: 0.7,
            }}
          >
            <span>
              {filteredNotes.length} of {notes.length} notes
              {searchTerm && ` matching "${searchTerm}"`}
              {selectedTag && ` tagged with "${selectedTag}"`}
            </span>
            <span>{allTags.length} unique tags</span>
          </div>
        </div>
      )}
    </div>
  );
}

