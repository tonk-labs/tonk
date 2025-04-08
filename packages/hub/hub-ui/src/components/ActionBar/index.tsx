import * as Dialog from "@radix-ui/react-dialog";
import { BookOpenText, HomeIcon, NotebookPen, PackagePlus } from "lucide-react";
import React, { useState } from "react";
import { openExternal } from "../../ipc/app";
import { createApp } from "../../ipc/hub";
import { useEventStore } from "../../stores/eventStore";
import { useProjectStore } from "../../stores/projectStore";
import Button from "../Button";
import styles from "./ActionBar.module.css";
import IntegrationDialog from "./IntegrationDialog";
import TextInput from "../TextInput";

const ActionBar: React.FC = () => {
    const [appName, setAppName] = useState("");
    const [isOpen, setIsOpen] = useState(false);
    const [isIntegrationDialogOpen, setIsIntegrationDialogOpen] =
        useState(false);

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
            <Button
                variant="ghost"
                size="sm"
                shape="square"
                onClick={handleGoHome}
                tooltip="Home"
                tooltipPosition="bottom"
            >
                <HomeIcon className={styles.homeIcon} />
            </Button>
            <Dialog.Root open={isOpen} onOpenChange={setIsOpen}>
                <Dialog.Trigger asChild>
                    <Button
                        variant="ghost"
                        size="sm"
                        shape="square"
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
                                variant="ghost"
                                size="sm"
                                onClick={() => setIsOpen(false)}
                            >
                                Cancel
                            </Button>
                            <Button
                                variant="purple"
                                size="sm"
                                onClick={handleCreateApp}
                            >
                                Create
                            </Button>
                        </div>
                        <Dialog.Close asChild>
                            <div className={styles.closeButton}>
                                <Button
                                    variant="ghost"
                                    size="sm"
                                    shape="square"
                                    aria-label="Close"
                            >
                                ×
                                </Button>
                            </div>
                        </Dialog.Close>
                    </Dialog.Content>
                </Dialog.Portal>
            </Dialog.Root>
            <IntegrationDialog
                isOpen={isIntegrationDialogOpen}
                setIsOpen={setIsIntegrationDialogOpen}
            />
            <Button
                variant="ghost"
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
