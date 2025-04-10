import * as Dialog from "@radix-ui/react-dialog";
import React, { useState } from "react";
import { createApp } from "../../ipc/hub";
import { useEventStore } from "../../stores/eventStore";
import Button from "../Button";
import TextInput from "../TextInput";
import styles from "./ActionBar.module.css";

interface CreateAppDialogProps {
  close: () => void;
}

const CreateAppDialog: React.FC<CreateAppDialogProps> = ({
  close,
}) => {
  const [appName, setAppName] = useState("");

  const addEvent = useEventStore((state) => state.addEvent);
  const handleCreateApp = async () => {
    if (appName.trim()) {
      await createApp(appName.trim());
      addEvent({
        appName: appName.trim(),
        timestamp: Date.now(),
        type: "init",
      });
      setAppName("");
      close();
    }
  };
  return (
    <Dialog.Portal>
      <Dialog.Overlay className={styles.dialogOverlay} />
      <Dialog.Content className={styles.dialogContent}>
        <Dialog.Title className={styles.dialogTitle}>
          Create New App
        </Dialog.Title>
        <Dialog.Description className={styles.dialogDescription}>
          Enter a name for your new app.
        </Dialog.Description>
        <TextInput
          className={styles.input}
          value={appName}
          onChange={(e) => setAppName(e.target.value)}
          placeholder="App name"
        />
        <div className={styles.dialogButtons}>
          <Button color="ghost" size="sm" onClick={close}>
            Cancel
          </Button>
          <Button color="green" size="sm" onClick={handleCreateApp}>
            Create
          </Button>
        </div>
        <Dialog.Close asChild>
          <Button
            color="ghost"
            size="sm"
            shape="square"
            aria-label="Close"
            className={styles.closeButton}
          >
            ×
          </Button>
        </Dialog.Close>
      </Dialog.Content>
    </Dialog.Portal>
  );
};

export default CreateAppDialog;
