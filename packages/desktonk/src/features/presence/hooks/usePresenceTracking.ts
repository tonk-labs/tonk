import { useEffect } from 'react';
import throttle from 'lodash.throttle';
import { usePresence } from '../stores/presenceStore';

const THROTTLE_MS = 4 * 60 * 1000; // Update every 4 minutes (before 5 min timeout)

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
