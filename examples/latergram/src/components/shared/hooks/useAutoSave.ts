import { useState, useEffect, useCallback, useRef } from 'react';

interface UseAutoSaveOptions {
  content: string;
  onSave: (content: string) => Promise<boolean>;
  debounceMs?: number;
  enabled?: boolean;
}

interface UseAutoSaveReturn {
  isSaving: boolean;
  lastSaved: Date | null;
  error: string | null;
  hasChanges: boolean;
}

export function useAutoSave({
  content,
  onSave,
  debounceMs = 1000,
  enabled = true,
}: UseAutoSaveOptions): UseAutoSaveReturn {
  const [isSaving, setIsSaving] = useState(false);
  const [lastSaved, setLastSaved] = useState<Date | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [originalContent, setOriginalContent] = useState(content);
  const timeoutRef = useRef<NodeJS.Timeout>(null);

  const hasChanges = content !== originalContent;

  const save = useCallback(async () => {
    if (!enabled || !hasChanges) return;

    setIsSaving(true);
    setError(null);

    try {
      await onSave(content);
      setLastSaved(new Date());
      setOriginalContent(content);
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Failed to save';
      setError(errorMsg);
      console.error('Auto-save failed:', err);
    } finally {
      setIsSaving(false);
    }
  }, [content, onSave, enabled, hasChanges]);

  // Debounced auto-save
  useEffect(() => {
    if (!enabled || !hasChanges) return;

    // Clear existing timeout
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
    }

    // Set new timeout
    timeoutRef.current = setTimeout(() => {
      save();
    }, debounceMs);

    // Cleanup
    return () => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, [content, save, debounceMs, enabled, hasChanges]);

  // Update original content when it changes externally (e.g., file loaded)
  useEffect(() => {
    if (!hasChanges && content !== originalContent) {
      setOriginalContent(content);
    }
  }, [content, originalContent, hasChanges]);

  return {
    isSaving,
    lastSaved,
    error,
    hasChanges,
  };
}

// Helper to format time since last save
export function formatTimeSince(date: Date | null): string {
  if (!date) return 'Never';

  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const seconds = Math.floor(diff / 1000);

  if (seconds < 5) return 'Just now';
  if (seconds < 60) return `${seconds} seconds ago`;

  const minutes = Math.floor(seconds / 60);
  if (minutes === 1) return '1 minute ago';
  if (minutes < 60) return `${minutes} minutes ago`;

  const hours = Math.floor(minutes / 60);
  if (hours === 1) return '1 hour ago';
  if (hours < 24) return `${hours} hours ago`;

  const days = Math.floor(hours / 24);
  if (days === 1) return '1 day ago';
  return `${days} days ago`;
}