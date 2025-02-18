import React, { useEffect, useState } from "react";
import { SyncEngine } from "../lib/sync-engine";

export const Home = () => {
  const [message, setMessage] = useState('');
  const [engineReady, setEngineReady] = useState(false);
  const [docCreated, setDocCreated] = useState(false);
  const [docContent, setDocContent] = useState('');

  const syncEngineRef = React.useRef<SyncEngine | null>(null);

  // Initialize the sync engine once when component mounts
  useEffect(() => {
    let mounted = true;

    const engine = new SyncEngine({
      port: 9000,
      onSync: async (docId) => {
        if (!mounted) return;
        console.log(`Document ${docId} synced`);
        const doc = await engine.getDocument(docId);
        if (doc && mounted) {
          console.log('Sync update - new content:', doc.content);
          setDocContent(doc.content);
        }
      },
      onError: (error) => console.error('Sync error:', error)
    });

    syncEngineRef.current = engine;

    const initEngine = async () => {
      try {
        await syncEngineRef.current?.init();
        if (mounted) setEngineReady(true);
        const existingDoc = await syncEngineRef.current?.getDocument('test-doc');
        if (!existingDoc) {
          await syncEngineRef.current?.createDocument('test-doc', {
            content: 'Initial content'
          });
        }
        setDocCreated(true);

        // Get and display initial document content
        const doc = await syncEngineRef.current?.getDocument('test-doc');
        if (mounted) {
          setDocContent(doc.content);
        }
      } catch (error) {
        console.error('Failed to initialise sync engine:', error);
      }
    };

    initEngine();

    return () => {
      mounted = false;
      syncEngineRef.current?.close();
    };
  }, []);

  const updateDocument = async () => {
    if (!engineReady || !docCreated) {
      console.warn('Engine not ready or document not created yet');
      return;
    }

    try {
      await syncEngineRef.current?.updateDocument('test-doc', doc => {
        doc.content = message;
      });

      // Immediately get and display the updated content
      const updatedDoc = await syncEngineRef.current?.getDocument('test-doc');
      if (updatedDoc) {
        console.log('Local update - new content:', updatedDoc.content);
        setDocContent(updatedDoc.content);
      }
    } catch (error) {
      console.error('Error updating document:', error);
    }
  };

  return (
    <div className="p-4">
      <h1 className="text-2xl font-bold mb-4">TinyFoot App</h1>
      <div className="space-y-4">
        <div>Status: {engineReady ? 'Ready' : 'Initializing...'}</div>
        <input
          type="text"
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          className="border p-2 rounded"
          placeholder="Enter message"
          disabled={!engineReady || !docCreated}
        />
        <button
          onClick={updateDocument}
          className="bg-blue-500 text-white px-4 py-2 rounded"
          disabled={!engineReady || !docCreated}
        >
          Update Document
        </button>

        <div className="mt-8">
          <h2 className="text-xl font-semibold mb-2">Document Content:</h2>
          <div className="p-4 border rounded bg-gray-50">
            {docContent || 'No content yet'}
          </div>
        </div>
      </div>
    </div>
  );
}
