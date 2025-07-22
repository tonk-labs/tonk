import React, { useState } from "react";
import { useTripStore } from "../stores/tripStore";
import { SharedNote } from "../types/travel";
import { Plus, Edit, Trash2, Save, X, Tag, User, Clock } from "lucide-react";
import styles from "./SharedNotes.module.css";

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

  const formatDate = (date: string) => {
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
      <div className={styles.searchAndFilter}>
        <div className={styles.searchFilterRow}>
          <div className={styles.searchInput}>
            <input
              type="text"
              placeholder="Search notes..."
              value={searchTerm}
              onChange={(e) => setSearchTerm(e.target.value)}
              className="form-input"
            />
          </div>
          <div className={styles.tagSelect}>
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
          <div className={styles.tagButtons}>
            <span className={styles.tagButtonsLabel}>Tags:</span>
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
          <div className={styles.emptyState}>
            <div className={styles.emptyStateIcon}>📝</div>
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
              <div key={note.id} className={`card ${styles.noteCard}`}>
                <div className={styles.noteHeader}>
                  <div style={{ flex: 1 }}>
                    <h3 className={styles.noteTitle}>{note.title}</h3>
                    <div className={styles.noteMeta}>
                      <div className={styles.noteMetaItem}>
                        <User size={14} />
                        <span>Created by {getMemberName(note.createdBy)}</span>
                      </div>
                      <div className={styles.noteMetaItem}>
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
                  <div className={styles.noteActions}>
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

                <div className={styles.noteContent}>
                  <div className={styles.noteText}>{note.content}</div>
                </div>

                {note.tags.length > 0 && (
                  <div className={styles.noteTags}>
                    <Tag size={14} style={{ opacity: 0.5 }} />
                    {note.tags.map((tag) => (
                      <span
                        key={tag}
                        onClick={() => setSelectedTag(tag)}
                        className={styles.noteTag}
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
        <div className={styles.notesSummary}>
          <div className={styles.notesSummaryContent}>
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
