import React, { useEffect, useState } from "react";
import styles from "./Main.module.css";

// Add type declaration for the electron API
declare global {
  interface Window {
    electronAPI: {
      getCurrentDocId: () => Promise<string>;
      getProjectDocIds: (projectPath: string) => Promise<string[]>;
      saveDocId: (projectPath: string, docId: string) => Promise<void>;
      launchApp: (path: string, docId: string) => Promise<void>;
    };
    require: (module: string) => any;
  }
}

const Main: React.FC = () => {
  const [docId, setDocId] = useState("");
  const [recentDocIds, setRecentDocIds] = useState<string[]>([]);
  const [selectedPath, setSelectedPath] = useState("");

  const handleSelectFolder = async () => {
    const { dialog } = window.require("@electron/remote");
    try {
      const result = await dialog.showOpenDialog({
        properties: ["openDirectory"],
      });

      if (!result.canceled) {
        const path = result.filePaths[0];
        setSelectedPath(path);
        // Load recent docIds for this project
        const ids = await window.electronAPI.getProjectDocIds(path);
        setRecentDocIds(ids || []);
      }
    } catch (error) {
      console.error("Error selecting folder:", error);
    }
  };

  const handleLaunchApp = async () => {
    if (!selectedPath || !docId) {
      alert("Please select both a folder and enter a document ID");
      return;
    }

    try {
      // Save the docId for this project
      await window.electronAPI.saveDocId(selectedPath, docId);
      // Launch the app with the selected docId
      await window.electronAPI.launchApp(selectedPath, docId);
    } catch (error) {
      console.error("Error launching app:", error);
      alert("Error launching app: " + error.message);
    }
  };

  return (
    <div className={styles.container}>
      <h1 className={styles.headerText}>Electron + Tonk!</h1>
      <button className={styles.button} onClick={handleSelectFolder}>
        Select App
      </button>

      {selectedPath && (
        <div className={styles.appConfig}>
          <p>Selected app: {selectedPath}</p>
          <div className={styles.inputContainer}>
            <input
              type="text"
              value={docId}
              onChange={(e) => setDocId(e.target.value)}
              placeholder="Enter Document ID"
              className={styles.input}
            />
          </div>

          {recentDocIds.length > 0 && (
            <div className={styles.recentIds}>
              <h3>Recent Document IDs:</h3>
              <ul>
                {recentDocIds.map((id) => (
                  <li key={id} onClick={() => setDocId(id)}>
                    {id}
                  </li>
                ))}
              </ul>
            </div>
          )}

          <button
            className={styles.launchButton}
            onClick={handleLaunchApp}
            disabled={!docId}
          >
            Launch App
          </button>
        </div>
      )}
    </div>
  );
};

export default Main;
