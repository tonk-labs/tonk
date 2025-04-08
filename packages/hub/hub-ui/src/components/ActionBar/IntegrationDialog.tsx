import * as Dialog from "@radix-ui/react-dialog";
import { PackagePlus } from "lucide-react";
import React from "react";
import Button from "../Button";
import styles from "./ActionBar.module.css";

interface IntegrationDialogProps {
    isOpen: boolean;
    setIsOpen: (open: boolean) => void;
}

const IntegrationDialog: React.FC<IntegrationDialogProps> = ({
    isOpen,
    setIsOpen,
}) => {
    const handleAddIntegration = () => {
        // TODO: Implement integration addition logic
        setIsOpen(false);
    };

    return (
        <Dialog.Root open={isOpen} onOpenChange={setIsOpen}>
            <Dialog.Trigger asChild>
                <Button
                    variant="ghost"
                    size="sm"
                    shape="square"
                    title="Add Integration"
                    tooltip="Add Integration"
                    tooltipPosition="bottom"
                >
                    <PackagePlus size={20} />
                </Button>
            </Dialog.Trigger>
            <Dialog.Portal>
                <Dialog.Overlay className={styles.dialogOverlay} />
                <Dialog.Content className={styles.dialogContent}>
                    <Dialog.Title className={styles.dialogTitle}>
                        Add Integration
                    </Dialog.Title>
                    <Dialog.Description className={styles.dialogDescription}>
                        Select an integration to add to your app.
                    </Dialog.Description>
                    {/* Add integration selection UI here */}
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
                            onClick={handleAddIntegration}
                        >
                            Add Integration
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
    );
};

export default IntegrationDialog;
