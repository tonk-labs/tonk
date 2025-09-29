// This script will forcibly unregister any service workers
// It's included in the development HTML to help clean up previous SW registrations

(function cleanupServiceWorkers() {
  if ('serviceWorker' in navigator) {
    console.log('SW-CLEANUP: Checking for service workers to remove...');
    navigator.serviceWorker.getRegistrations()
      .then(registrations => {
        if (registrations.length > 0) {
          console.log(`SW-CLEANUP: Found ${registrations.length} service worker(s) to remove`);
          return Promise.all(registrations.map(registration => {
            console.log('SW-CLEANUP: Unregistering service worker');
            return registration.unregister();
          }));
        } else {
          console.log('SW-CLEANUP: No service workers found');
        }
      })
      .then(() => {
        console.log('SW-CLEANUP: Service worker cleanup complete');
      })
      .catch(error => {
        console.error('SW-CLEANUP: Error during cleanup', error);
      });
  }
})(); 