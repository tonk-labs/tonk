import React, { useState } from "react";
import { NotebookPen, PackagePlus, BookOpenText, HomeIcon } from "lucide-react";
import * as Dialog from "@radix-ui/react-dialog";
import styles from "./ActionBar.module.css";
import { openExternal } from "../../ipc/app";
import { createApp } from "../../ipc/hub";
import { useEventStore } from "../../stores/eventStore";
import Button from "../Button";
import { useProjectStore } from "../../stores/projectStore";

const ActionBar: React.FC = () => {
  const [appName, setAppName] = useState("");
  const [isOpen, setIsOpen] = useState(false);

  const { setSelectedItem } = useProjectStore();
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

const handleGoHome = () => {
    setSelectedItem(null);
  };

  return (
    <div className={styles.actionBar}>
      <Button variant="ghost" size="sm" shape="square" onClick={handleGoHome} tooltip="Home" tooltipPosition="bottom">
          <HomeIcon className={styles.homeIcon} />
        </Button>
      <Dialog.Root open={isOpen} onOpenChange={setIsOpen}>
        <Dialog.Trigger asChild>
          <Button variant="ghost" size="sm" shape="square" title="Create App" tooltip="Create App" tooltipPosition="bottom">
            <NotebookPen size={20} />
          </Button>
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
      <Button variant="ghost" size="sm" shape="square" title="Add Integration" tooltip="Add Integration" tooltipPosition="bottom">
        <PackagePlus size={20} />
      </Button>
      <Button variant="ghost" size="sm" shape="square" title="Documentation" onClick={openDocs} tooltip="Documentation" tooltipPosition="bottom">
        <BookOpenText size={20} />
      </Button>
    </div>
  );
};

export default ActionBar;
