import { useState, useRef, useEffect, type KeyboardEvent } from 'react';
import { useEditorStore } from '@/features/editor/stores/editorStore';
import './editable-title.css';

const MAX_TITLE_LENGTH = 100;

export function EditableTitle() {
  const title = useEditorStore(state => state.metadata.title);
  const setTitle = useEditorStore(state => state.setTitle);
  const [isEditing, setIsEditing] = useState(false);
  const [localTitle, setLocalTitle] = useState(title);
  const inputRef = useRef<HTMLInputElement>(null);

  // Sync local state when title changes externally
  useEffect(() => {
    if (!isEditing) {
      setLocalTitle(title);
    }
  }, [title, isEditing]);

  // Auto-focus and select text when entering edit mode
  useEffect(() => {
    if (isEditing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [isEditing]);

  const validateAndSave = () => {
    let validated = localTitle.trim();

    // Enforce max length
    if (validated.length > MAX_TITLE_LENGTH) {
      validated = validated.substring(0, MAX_TITLE_LENGTH);
    }

    // Revert to "Untitled" if empty
    if (validated === '') {
      validated = 'Untitled';
    }

    setTitle(validated);
    setLocalTitle(validated);
    setIsEditing(false);
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      validateAndSave();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      setLocalTitle(title);
      setIsEditing(false);
    }
  };

  const handleClick = () => {
    if (!isEditing) {
      setIsEditing(true);
    }
  };

  const handleBlur = () => {
    validateAndSave();
  };

  if (isEditing) {
    return (
      <input
        ref={inputRef}
        type="text"
        value={localTitle}
        onChange={(e) => setLocalTitle(e.target.value)}
        onKeyDown={handleKeyDown}
        onBlur={handleBlur}
        className="editable-title-input"
        aria-label="Document title"
        maxLength={MAX_TITLE_LENGTH + 10} // Allow some buffer for user experience
      />
    );
  }

  return (
    <h1
      onClick={handleClick}
      className="editable-title-display"
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          handleClick();
        }
      }}
    >
      {title}
    </h1>
  );
}
