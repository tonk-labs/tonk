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

const CreateAppDialog: React.FC<CreateAppDialogProps> = ({ close }) => {
  const [appName, setAppName] = useState("");
  const [directoryName, setDirectoryName] = useState("");

  const addEvent = useEventStore((state) => state.addEvent);

  const formatDirectoryName = (name: string): string => {
    // Replace spaces with hyphens
    let formatted = name.replace(/\s+/g, "-");
    // Remove invalid characters
    formatted = formatted.replace(/[/\\:.;*!?"'<>|]/g, "").toLowerCase();
    return formatted;
  };

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const value = e.target.value;
    setAppName(value);
    setDirectoryName(formatDirectoryName(value));
  };

  const handleCreateApp = async () => {
    if (directoryName) {
      await createApp(directoryName);
      addEvent({
        appName: directoryName,
        timestamp: Date.now(),
        type: "init",
      });
      setAppName("");
      setDirectoryName("");
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
          onChange={handleInputChange}
          placeholder="App name"
        />
        <div className={styles.directoryPreview}>
          {directoryName && (
            <>
              Directory name:{" "}
              <span className={styles.previewName}>{directoryName}</span>
            </>
          )}
        </div>
        <div className={styles.dialogButtons}>
          <Button color="ghost" size="sm" onClick={close}>
            Cancel
          </Button>
          <Button
            color="green"
            size="sm"
            onClick={handleCreateApp}
            disabled={!directoryName}
          >
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
