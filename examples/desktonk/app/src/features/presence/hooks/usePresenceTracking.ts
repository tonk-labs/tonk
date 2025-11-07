import { useEffect } from 'react';
import throttle from 'lodash.throttle';
import { usePresence } from '../stores/presenceStore';

const THROTTLE_MS = 1000; // Max 1 update per second

/**
 * Track user activity and update presence
 * Monitors: mousemove, keydown on document
 */
export const usePresenceTracking = () => {
  const { updatePresence } = usePresence();

  useEffect(() => {
    // Throttled update function
    const throttledUpdate = throttle(
      () => {
        updatePresence();
      },
      THROTTLE_MS,
      { leading: true, trailing: false }
    );

    // Event listeners
    const handleActivity = () => {
      throttledUpdate();
    };

    // Attach listeners
    document.addEventListener('mousemove', handleActivity);
    document.addEventListener('keydown', handleActivity);

    // Cleanup
    return () => {
      document.removeEventListener('mousemove', handleActivity);
      document.removeEventListener('keydown', handleActivity);
      throttledUpdate.cancel();
    };
  }, [updatePresence]);
};
