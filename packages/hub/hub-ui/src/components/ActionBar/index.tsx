import * as Dialog from "@radix-ui/react-dialog";
import { BookOpenText, HomeIcon, LogOut, NotebookPen, PackagePlus } from "lucide-react";
import React, { useState } from "react";
import { openExternal } from "../../ipc/app";
import { clearConfig, createApp } from "../../ipc/hub";
import { useEventStore } from "../../stores/eventStore";
import { useProjectStore } from "../../stores/projectStore";
import Button from "../Button";
import styles from "./ActionBar.module.css";
import TextInput from "../TextInput";

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

    const handleLogout = async () => {
        await clearConfig();
        window.location.reload();
    };

    return (
        <div className={styles.actionBar}>
            <Button

                size="sm"
                shape="square"
                color="ghost"
                onClick={handleGoHome}
                tooltip="Home"
                tooltipPosition="bottom"
            >
                <HomeIcon className={styles.homeIcon} />
            </Button>
            <Dialog.Root open={isOpen} onOpenChange={setIsOpen}>
                <Dialog.Trigger asChild>
                    <Button
                        size="sm"
                        shape="square"
                        color="ghost"
                        title="Create App"
                        tooltip="Create App"
                        tooltipPosition="bottom"
                    >
                        <NotebookPen size={20} />
                    </Button>
                </Dialog.Trigger>
                <Dialog.Portal>
                    <Dialog.Overlay className={styles.dialogOverlay} />
                    <Dialog.Content className={styles.dialogContent}>
                        <Dialog.Title className={styles.dialogTitle}>
                            Create New App
                        </Dialog.Title>
                        <Dialog.Description
                            className={styles.dialogDescription}
                        >
                            Enter a name for your new app.
                        </Dialog.Description>
                        <TextInput
                            className={styles.input}
                            value={appName}
                            onChange={(e) => setAppName(e.target.value)}
                            placeholder="App name"
                        />
                        <div className={styles.dialogButtons}>
                            <Button
                                color="ghost"
                                size="sm"
                                onClick={() => setIsOpen(false)}
                            >
                                Cancel
                            </Button>
                            <Button
                                color="green"
                                size="sm"
                                onClick={handleCreateApp}
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
            </Dialog.Root>
            <Button
                color="ghost"
                size="sm"
                shape="square"
                title="Documentation"
                onClick={openDocs}
                tooltip="Documentation"
                tooltipPosition="bottom"
            >
                <BookOpenText size={20} />
            </Button>
        </div>
    );
};

export default ActionBar;
