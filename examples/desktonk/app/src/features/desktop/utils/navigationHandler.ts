import { showError } from '../../../lib/notifications';

type NavigationHandler = (path: string) => void;

let navigationHandler: NavigationHandler | null = null;

export function setNavigationHandler(handler: NavigationHandler | null): void {
  navigationHandler = handler;
}

export function navigate(path: string): void {
  if (navigationHandler) {
    navigationHandler(path);
  } else {
    console.error('Navigation handler not set. Cannot navigate to:', path);
    showError('Navigation system not initialized. Please refresh the page.');
  }
}
