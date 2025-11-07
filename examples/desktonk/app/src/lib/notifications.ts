/**
 * Simple notification system for user-facing error messages.
 * Shows temporary toast notifications without external dependencies.
 */

type NotificationType = 'error' | 'warning' | 'success' | 'info';

interface NotificationOptions {
  duration?: number; // milliseconds, default 5000
  type?: NotificationType;
}

const DEFAULT_DURATION = 5000;

/**
 * Shows a toast notification to the user.
 * Creates a temporary DOM element that auto-dismisses after the specified duration.
 * Notification can be manually dismissed by clicking on it.
 *
 * @param message - Text to display in the notification
 * @param options - Configuration options for the notification
 * @param options.duration - How long to show notification in ms (default: 5000)
 * @param options.type - Visual style: 'error', 'warning', 'success', or 'info' (default: 'info')
 *
 * @example
 * ```typescript
 * showNotification('File saved successfully', { type: 'success' });
 * showNotification('Connection lost', { type: 'error', duration: 7000 });
 * ```
 */
export function showNotification(message: string, options: NotificationOptions = {}): void {
  const { duration = DEFAULT_DURATION, type = 'info' } = options;

  // Create toast element
  const toast = document.createElement('div');
  toast.className = `notification notification-${type}`;
  toast.textContent = message;

  // Style the toast
  Object.assign(toast.style, {
    position: 'fixed',
    bottom: '20px',
    right: '20px',
    padding: '12px 20px',
    borderRadius: '8px',
    backgroundColor: getBackgroundColor(type),
    color: '#fff',
    fontSize: '14px',
    fontFamily: 'system-ui, -apple-system, sans-serif',
    boxShadow: '0 4px 12px rgba(0, 0, 0, 0.15)',
    zIndex: '10000',
    maxWidth: '400px',
    animation: 'slideIn 0.3s ease-out',
    cursor: 'pointer',
  });

  // Add animation styles if not already present
  if (!document.getElementById('notification-styles')) {
    const styleSheet = document.createElement('style');
    styleSheet.id = 'notification-styles';
    styleSheet.textContent = `
      @keyframes slideIn {
        from {
          transform: translateX(400px);
          opacity: 0;
        }
        to {
          transform: translateX(0);
          opacity: 1;
        }
      }
      @keyframes slideOut {
        from {
          transform: translateX(0);
          opacity: 1;
        }
        to {
          transform: translateX(400px);
          opacity: 0;
        }
      }
    `;
    document.head.appendChild(styleSheet);
  }

  // Add to DOM
  document.body.appendChild(toast);

  // Auto-dismiss
  const timeoutId = setTimeout(() => {
    dismiss(toast);
  }, duration);

  // Allow manual dismissal by clicking
  toast.addEventListener('click', () => {
    clearTimeout(timeoutId);
    dismiss(toast);
  });
}

function dismiss(toast: HTMLElement): void {
  toast.style.animation = 'slideOut 0.3s ease-out';
  setTimeout((): void => {
    toast.remove();
  }, 300);
}

const NOTIFICATION_COLORS: Record<NotificationType, string> = {
  error: '#dc2626',   // red-600
  warning: '#f59e0b', // amber-500
  success: '#16a34a', // green-600
  info: '#2563eb',    // blue-600
};

function getBackgroundColor(type: NotificationType): string {
  return NOTIFICATION_COLORS[type];
}

/**
 * Shows an error notification in red.
 * Use for critical failures that require user attention.
 *
 * @param message - Error message to display
 * @param duration - Optional duration in ms (default: 5000)
 *
 * @example
 * ```typescript
 * showError('Failed to save file', 7000);
 * ```
 */
export function showError(message: string, duration?: number): void {
  showNotification(message, { type: 'error', duration });
}

/**
 * Shows a warning notification in amber.
 * Use for important information that's not critical.
 *
 * @param message - Warning message to display
 * @param duration - Optional duration in ms (default: 5000)
 *
 * @example
 * ```typescript
 * showWarning('Connection unstable');
 * ```
 */
export function showWarning(message: string, duration?: number): void {
  showNotification(message, { type: 'warning', duration });
}

/**
 * Shows a success notification in green.
 * Use for confirming successful operations.
 *
 * @param message - Success message to display
 * @param duration - Optional duration in ms (default: 5000)
 *
 * @example
 * ```typescript
 * showSuccess('File saved successfully');
 * ```
 */
export function showSuccess(message: string, duration?: number): void {
  showNotification(message, { type: 'success', duration });
}
