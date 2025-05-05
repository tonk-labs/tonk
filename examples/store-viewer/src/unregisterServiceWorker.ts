/**
 * Utility function to forcibly unregister all service workers
 * Call this function from your browser console to clean up stuck service workers:
 * 
 * import('./unregisterServiceWorker.ts').then(m => m.unregisterAllServiceWorkers())
 */
export function unregisterAllServiceWorkers(): Promise<void> {
  if (!('serviceWorker' in navigator)) {
    console.log('Service workers not supported in this browser');
    return Promise.resolve();
  }

  return navigator.serviceWorker.getRegistrations()
    .then(registrations => {
      const unregisterPromises = registrations.map(registration => {
        const scope = registration.scope;
        return registration.unregister()
          .then(success => {
            if (success) {
              console.log(`Successfully unregistered service worker with scope: ${scope}`);
            } else {
              console.warn(`Failed to unregister service worker with scope: ${scope}`);
            }
          });
      });
      
      return Promise.all(unregisterPromises)
        .then(() => {
          if (registrations.length === 0) {
            console.log('No service workers found to unregister');
          } else {
            console.log(`Attempted to unregister ${registrations.length} service worker(s)`);
            console.log('Please refresh the page for changes to take effect');
          }
        });
    })
    .catch(error => {
      console.error('Error while unregistering service workers:', error);
    });
}

// Auto-execute if this file is loaded directly
if (import.meta.url === document.currentScript?.getAttribute('src')) {
  unregisterAllServiceWorkers();
} 