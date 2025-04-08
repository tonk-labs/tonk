import * as Dialog from "@radix-ui/react-dialog";
import { PackagePlus } from "lucide-react";
import React from "react";
import { useIntegrations } from "../../hooks/useIntegrations";
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
    const {
        integrations,
        selectedIntegration,
        isLoading,
        error,
        selectIntegration,
    } = useIntegrations();

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

                    {error && <div className={styles.error}>{error}</div>}

                    <div className={styles.integrationList}>
                        {integrations.map((integration) => (
                            <div
                                key={integration.name}
                                className={`${styles.integrationItem} ${
                                    selectedIntegration === integration.name
                                        ? styles.selected
                                        : ""
                                } ${integration.isInstalled ? styles.installed : ""}`}
                                onClick={() =>
                                    selectIntegration(integration.name)
                                }
                            >
                                <div className={styles.integrationName}>
                                    {integration.name}
                                </div>
                                {integration.isInstalled && (
                                    <span className={styles.installedBadge}>
                                        Installed
                                    </span>
                                )}
                            </div>
                        ))}
                    </div>

                    <div className={styles.dialogButtons}>
                        <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setIsOpen(false)}
                            disabled={isLoading}
                        >
                            Cancel
                        </Button>
                        <Button
                            variant="purple"
                            size="sm"
                            onClick={() => {}}
                            disabled={isLoading || !selectedIntegration}
                        >
                            {isLoading ? "Adding..." : "Add Integration"}
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
