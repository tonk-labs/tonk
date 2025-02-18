
<<<<<<< HEAD
=======
    let mounted = true;

    const engine = new SyncEngine({
      port: 9000,
      onSync: async (docId) => {
        if (!mounted) return;
        console.log(`Document ${docId} synced`);
        const doc = await engine.getDocument(docId);
        if (doc && mounted) {
>>>>>>> Snippet
