import { TonkToolError } from './core.mjs';

// Public test writes explicitly sync once. A failed push must not be described
// as an unapplied edit or cause the evaluator to be replayed.
export function hostedBackend(backend) {
  return {
    capabilities: backend.capabilities,
    uiRead: backend.uiRead,
    webOrigin: 'https://tonk.foundation',
    async call(name, args, signal) {
      const result = await backend.call(name, args, signal);
      if (name !== 'tonk_apply' && !(name === 'tonk_install_library' && result.committed === true)) return result;
      try {
        const sync = await backend.push(signal);
        return { ...result, sync,
          scope: 'The change committed and the remote push succeeded. Rendering is not confirmed. Read back the record; do not repeat this write.',
        };
      } catch {
        throw new TonkToolError('The edit committed locally, but remote synchronization could not be confirmed. Query the space before deciding what to do; do not repeat the edit automatically.');
      }
    },
  };
}
