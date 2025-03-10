import React, { useState, useEffect } from 'react';
import {X} from 'lucide-react';
import styles from './Settings.module.css';

interface SettingsProps {
  nav: (pageName: string) => void
}

const Settings: React.FC<SettingsProps> = (props: SettingsProps) => {
  const [apiKey, setApiKey] = useState<string>('');
  const [isSaving, setIsSaving] = useState<boolean>(false);
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(true);

  // Load API key when component mounts
  useEffect(() => {
    const fetchApiKey = async () => {
      try {
        const response = await fetch('http://localhost:3000/api-key');
        if (response.ok) {
          const data = await response.json();
          if (data.apiKey) {
            setApiKey(data.apiKey);
          }
        }
      } catch (error) {
        console.error('Failed to load API key:', error);
      } finally {
        setIsLoading(false);
      }
    };

    fetchApiKey();
  }, []);

  const handleSave = async () => {
    setIsSaving(true);
    setSaveStatus(null);
    
    try {
      // Send API key to the server
      const response = await fetch('http://localhost:3000/api-key', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ apiKey }),
      });
      
      if (!response.ok) {
        throw new Error(`Server responded with status: ${response.status}`);
      }
      
      const data = await response.json();
      setSaveStatus('Settings saved successfully');
      console.log('API key saved:', data);
    } catch (error) {
      console.error('Failed to save API key:', error);
      setSaveStatus('Failed to save settings');
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className={styles.container}>
      <h2 className={styles.title}>Settings</h2>
      
      <div className={styles.formGroup}>
        <label 
          htmlFor="claudeApiKey" 
          className={styles.label}
        >
          Claude API Key
        </label>
        <input
          id="claudeApiKey"
          type="text"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder={isLoading ? "Loading..." : "Enter your Claude API key"}
          className={styles.input}
          disabled={isLoading}
        />
      </div>
      
      <button
        onClick={handleSave}
        disabled={isSaving || isLoading}
        className={isSaving || isLoading ? styles.buttonDisabled : styles.buttonPrimary}
      >
        {isLoading ? 'Loading...' : isSaving ? 'Saving...' : 'Save Settings'}
      </button>
      
      {saveStatus && (
        <div className={saveStatus.includes('success') ? styles.statusSuccess : styles.statusError}>
          {saveStatus}
        </div>
      )}

      <p 
        onClick={() => props.nav("Chat")}
        className={styles.backLink}
      >
        Go back
      </p>
    </div>
  );
};

export default Settings;
