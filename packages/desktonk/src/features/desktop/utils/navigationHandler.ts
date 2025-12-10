type NavigationHandler = (path: string) => void;
type ToastHandler = (options: {
  title: string;
  severity: 'error' | 'warning' | 'success';
}) => void;

let navigationHandler: NavigationHandler | null = null;
let toastHandler: ToastHandler | null = null;

export function setNavigationHandler(
  handler: NavigationHandler | null,
  toast?: ToastHandler | null
): void {
  navigationHandler = handler;
  toastHandler = toast || null;
}

export function navigate(path: string): void {
  if (navigationHandler) {
    navigationHandler(path);
  } else {
    console.error('Navigation handler not set. Cannot navigate to:', path);
    if (toastHandler) {
      toastHandler({
        title: 'Navigation system not initialized. Please refresh the page.',
        severity: 'error',
      });
    }
  }
}
