import React, { useState } from "react";
import { NotebookPen, PackagePlus, BookOpenText } from "lucide-react";
import * as Dialog from "@radix-ui/react-dialog";
import styles from "./ActionBar.module.css";
import { openExternal } from "../../ipc/app";
import { createApp } from "../../ipc/hub";
import { useEventStore } from "../../stores/eventStore";

const ActionBar: React.FC = () => {
  const [appName, setAppName] = useState("");
  const [isOpen, setIsOpen] = useState(false);
  const addEvent = useEventStore((state) => state.addEvent);

  const openDocs = () => {
    openExternal("https://tonk-labs.github.io/tonk/");
  };

  const handleCreateApp = async () => {
    if (appName.trim()) {
      await createApp(appName.trim());
      addEvent({
        appName: appName.trim(),
        timestamp: Date.now(),
        type: "init",
      });
      setAppName("");
      setIsOpen(false);
    }
  };

  return (
    <div className={styles.actionBar}>
      <Dialog.Root open={isOpen} onOpenChange={setIsOpen}>
        <Dialog.Trigger asChild>
          <button className={styles.actionButton} title="Create App">
            <NotebookPen size={20} />
          </button>
        </Dialog.Trigger>
        <Dialog.Portal>
          <Dialog.Overlay className={styles.dialogOverlay} />
          <Dialog.Content className={styles.dialogContent}>
            <Dialog.Title className={styles.dialogTitle}>
              Create New App
            </Dialog.Title>
            <Dialog.Description className={styles.dialogDescription}>
              Enter a name for your new app.
            </Dialog.Description>
            <input
              className={styles.input}
              value={appName}
              onChange={(e) => setAppName(e.target.value)}
              placeholder="App name"
            />
            <div className={styles.dialogButtons}>
              <button
                className={styles.cancelButton}
                onClick={() => setIsOpen(false)}
              >
                Cancel
              </button>
              <button className={styles.createButton} onClick={handleCreateApp}>
                Create
              </button>
            </div>
            <Dialog.Close asChild>
              <button className={styles.closeButton} aria-label="Close">
                ×
              </button>
            </Dialog.Close>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
      <button className={styles.actionButton} title="Add Integration">
        <PackagePlus size={20} />
      </button>
      <button
        className={styles.actionButton}
        title="Documentation"
        onClick={openDocs}
      >
        <BookOpenText size={20} />
      </button>
    </div>
  );
};

export default ActionBar;
