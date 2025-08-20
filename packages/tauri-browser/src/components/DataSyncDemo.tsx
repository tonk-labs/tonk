import { useState, useEffect } from 'react';
import { P2PSync } from '../lib/p2p-sync';
import { readDoc, writeDoc, ls, getSyncEngine } from '@tonk/keepsync';

interface DataSyncDemoProps {
  p2pSync: P2PSync | null;
}

export function DataSyncDemo({ p2pSync }: DataSyncDemoProps) {
  const [messages, setMessages] = useState<any[]>([]);
  const [newMessage, setNewMessage] = useState('');
  const [files, setFiles] = useState<string[]>([]);
  const [isConfigured, setIsConfigured] = useState(false);

  useEffect(() => {
    if (!p2pSync || isConfigured) return;

    // Check if the SyncEngine is properly configured and ready
    const checkSyncEngine = async () => {
      const syncEngine = getSyncEngine();
      if (syncEngine) {
        try {
          await syncEngine.whenReady();
          setIsConfigured(true);
          loadData();

          // Set up interval to refresh data more frequently
          console.log('Setting up polling interval every 500ms');
          const interval = setInterval(() => {
            console.log('Polling interval triggered, calling loadData');
            loadData();
          }, 500);
          return () => {
            console.log('Cleaning up polling interval');
            clearInterval(interval);
          };
        } catch (error) {
          console.error('Failed to wait for sync engine:', error);
        }
      }
    };

    checkSyncEngine();
  }, [p2pSync, isConfigured]);

  const loadData = async () => {
    if (!isConfigured) return;

    try {
      // Load messages
      console.log('Loading messages from /messages...');
      const messagesDoc = await readDoc('/messages');
      console.log('Messages document:', messagesDoc);

      if (messagesDoc && (messagesDoc as any).messages) {
        console.log('Found messages:', (messagesDoc as any).messages);
        setMessages((messagesDoc as any).messages);
      } else {
        console.log('No messages found in document');
      }

      // Load file list
      console.log('Loading file list from /...');
      const fileList = await ls('/');
      console.log('File list:', fileList);

      if (fileList && fileList.children) {
        const fileNames = Object.keys(fileList.children);
        console.log('Found files:', fileNames);
        setFiles(fileNames);
      } else {
        console.log('No files found');
      }
    } catch (error) {
      console.error('Failed to load data:', error);
    }
  };

  const sendMessage = async () => {
    if (!isConfigured || !newMessage.trim()) return;

    try {
      console.log('Sending message:', newMessage);

      // Read current messages
      console.log('Reading current messages...');
      const messagesDoc = (await readDoc('/messages')) || { messages: [] };
      console.log('Current messages doc:', messagesDoc);

      // Create a clean copy of existing messages (without Automerge metadata)
      const existingMessages = ((messagesDoc as any).messages || []).map(
        (msg: any) => ({
          id: msg.id,
          text: msg.text,
          timestamp: msg.timestamp,
          sender: msg.sender,
        })
      );

      // Add new message
      const updatedMessages = [
        ...existingMessages,
        {
          id: Date.now(),
          text: newMessage,
          timestamp: new Date().toISOString(),
          sender: 'local',
        },
      ];

      console.log('Writing updated messages:', updatedMessages);

      // Write back to sync
      await writeDoc('/messages', { messages: updatedMessages });

      console.log('Message written successfully');

      setNewMessage('');
      setTimeout(loadData, 100);
    } catch (error) {
      console.error('Failed to send message:', error);
    }
  };

  const createTestFile = async () => {
    if (!isConfigured) return;

    try {
      const fileName = `/test-${Date.now()}`;
      await writeDoc(fileName, {
        created: new Date().toISOString(),
        data: { test: true, random: Math.random() },
      });
      setTimeout(loadData, 100);
    } catch (error) {
      console.error('Failed to create test file:', error);
    }
  };

  if (!p2pSync) {
    return (
      <div className="data-sync-demo">
        <h2>Data Sync Demo</h2>
        <p>Initialize P2P first to test data synchronization with Iroh</p>
      </div>
    );
  }

  if (!isConfigured) {
    return (
      <div className="data-sync-demo">
        <h2>Data Sync Demo</h2>
        <p>Configuring keepsync with Iroh network adapter...</p>
      </div>
    );
  }

  return (
    <div className="data-sync-demo">
      <h2>Data Sync Demo</h2>

      <div className="files-section">
        <h3>Synced Files ({files.length})</h3>
        <button onClick={createTestFile}>Create Test File</button>
        <ul>
          {files.map(file => (
            <li key={file}>{file}</li>
          ))}
        </ul>
      </div>

      <div className="messages-section">
        <h3>Shared Messages ({messages.length})</h3>
        <div className="messages-list">
          {messages.map((msg, index) => (
            <div key={msg.id || index} className="message">
              <strong>{msg.sender}:</strong> {msg.text}
              <small> ({new Date(msg.timestamp).toLocaleTimeString()})</small>
            </div>
          ))}
        </div>

        <div className="message-input">
          <input
            type="text"
            value={newMessage}
            onChange={e => setNewMessage(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && sendMessage()}
            placeholder="Type a message..."
          />
          <button onClick={sendMessage}>Send</button>
        </div>
      </div>
    </div>
  );
}
