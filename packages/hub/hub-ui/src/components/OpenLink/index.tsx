import React, { useState } from "react";
import { Text } from "..";
import styles from "./OpenLink.module.css";

const OpenLink: React.FC = () => {
  const [url, setUrl] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState("");

  const handleOpenLink = async () => {
    if (!url || !url.trim()) {
      setError("Please enter a valid URL");
      return;
    }

    // Add http:// prefix if not present
    let formattedUrl = url;
    if (!url.startsWith("http://") && !url.startsWith("https://")) {
      formattedUrl = "https://" + url;
    }

    try {
      setIsLoading(true);
      setError("");

      await window.electronAPI.openUrlInElectron(formattedUrl);
      setUrl(""); // Clear input after successful open
    } catch (err: any) {
      setError("Failed to open the URL: " + err.message);
    } finally {
      setIsLoading(false);
    }
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      handleOpenLink();
    }
  };

  return (
    <div className={styles.container}>
      <Text>Open Tonk App:</Text>
      <ul className={styles.inputList}>
        <li className={styles.inputItem}>
          <input
            type="text"
            className={styles.input}
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={handleKeyPress}
            placeholder="***.ngrok-free.app"
            disabled={isLoading}
          />
          <button
            className={styles.openButton}
            onClick={handleOpenLink}
            disabled={isLoading}
          >
            {isLoading ? "Opening..." : "Open"}
          </button>
        </li>
        {error && <li className={styles.errorItem}>{error}</li>}
      </ul>
    </div>
  );
};

export default OpenLink;
