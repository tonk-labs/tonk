import React from "react";
import { useProjectStore } from "../../stores/projectStore";
import styles from "./FileViewer.module.css";

const FileViewer: React.FC = () => {
  const { selectedItem, automergeContent, error } = useProjectStore();

  if (!selectedItem) {
    return (
      <div className={styles.container}>
        <div className={styles.emptyState}>No file selected</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className={styles.container}>
        <div className={styles.error}>
          <h3>Error</h3>
          <p>{error}</p>
        </div>
      </div>
    );
  }

  if (
    selectedItem.index.startsWith("stores/") &&
    selectedItem.index.endsWith(".automerge")
  ) {
    if (!automergeContent) {
      return (
        <div className={styles.container}>
          <div className={styles.loading}>Loading Automerge content...</div>
        </div>
      );
    }

    try {
      const parsedContent = JSON.parse(automergeContent);
      return (
        <div className={styles.container}>
          <div className={styles.header}>
            <h3>Automerge Document: {selectedItem.data.name}</h3>
          </div>
          <div className={styles.content}>
            <pre className={styles.jsonViewer}>
              {JSON.stringify(parsedContent, null, 2)}
            </pre>
          </div>
        </div>
      );
    } catch (e) {
      return (
        <div className={styles.container}>
          <div className={styles.error}>
            <h3>Invalid JSON Content</h3>
            <pre>{automergeContent}</pre>
          </div>
        </div>
      );
    }
  }

  return (
    <div className={styles.container}>
      <div className={styles.emptyState}>
        Select a store file to view its content
      </div>
    </div>
  );
};

export default FileViewer;
