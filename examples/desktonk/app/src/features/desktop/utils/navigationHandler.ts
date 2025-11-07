type NavigationHandler = (path: string) => void;

let navigationHandler: NavigationHandler | null = null;

export function setNavigationHandler(handler: NavigationHandler): void {
  navigationHandler = handler;
}

export function navigate(path: string): void {
  if (navigationHandler) {
    navigationHandler(path);
  } else {
    console.warn('Navigation handler not set. Navigation to:', path);
  }
}
