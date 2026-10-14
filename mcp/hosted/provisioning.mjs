// Seeds must be closed before checkpointing. Serialize both creation and
// authorization; a failed operation must not poison subsequent account setup.
export function checkpointedProvisioner({
  provision,
  checkpoint,
  retainedTenants,
  onFailure = (detail) => console.error(JSON.stringify(detail)),
}) {
  let pending = Promise.resolve();
  function enqueue(operation) {
    const result = pending.then(operation);
    pending = result.catch(() => {});
    return result;
  }

  return async () => {
    const account = await enqueue(provision);
    return {
      ...account,
      authorize(authorization) {
        return enqueue(async () => {
          const identity = await account.authorize(authorization);
          try {
            await checkpoint.save({ roots: new Set([account.tenantId, ...retainedTenants()]) });
          } catch (cause) {
            // Never log the upstream message, checkpoint contents or grants.
            const reasons = [
              'file-limit',
              'expanded-limit',
              'compressed-limit',
              'storage-rejected',
            ];
            onFailure({
              event: 'authorization-checkpoint-failed',
              reason: reasons.includes(cause?.checkpointReason)
                ? cause.checkpointReason
                : 'unavailable',
              status: Number.isInteger(cause?.checkpointStatus)
                ? cause.checkpointStatus
                : undefined,
            });
            throw Object.assign(Error('Checkpoint failed'), { authorizationStage: 'checkpoint' });
          }
          return identity;
        });
      },
    };
  };
}
