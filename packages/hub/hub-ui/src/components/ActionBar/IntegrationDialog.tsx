import * as Dialog from "@radix-ui/react-dialog";
import { PackagePlus } from "lucide-react";
import React, { useState } from "react";
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
        installedIntegrations,
    } = useIntegrations();
    const [isInstalling, setIsInstalling] = useState(false);

    const handleInstall = async () => {
        if (!selectedIntegration) return;

        try {
            setIsInstalling(true);
            const result =
                await window.electronAPI.installIntegration(
                    selectedIntegration
                );
            if (!result.success) {
                throw new Error(result.error);
            }
        } catch (err) {
            console.error("Failed to install integration:", err);
        } finally {
            setIsInstalling(false);
        }
    };

    // Filter available integrations to only show uninstalled ones
    const availableIntegrations = integrations.filter(
        (integration) => !integration.isInstalled
    );

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
                        Integrations are packages that extend the functionality
                        of your app.
                    </Dialog.Description>

                    {error && <div className={styles.error}>{error}</div>}

                    <div className={styles.integrationSections}>
                        {/* Installed Integrations Section */}
                        <div className={styles.integrationSection}>
                            <h3 className={styles.sectionTitle}>
                                Installed Integrations
                            </h3>
                            <div className={styles.integrationList}>
                                {installedIntegrations.map((integration) => (
                                    <div
                                        key={integration.name}
                                        className={styles.installedIntegration}
                                    >
                                        <div
                                            className={styles.integrationHeader}
                                        >
                                            <div
                                                className={
                                                    styles.integrationName
                                                }
                                            >
                                                {integration.name}
                                            </div>
                                            <div
                                                className={
                                                    styles.integrationVersion
                                                }
                                            >
                                                v{integration.version}
                                            </div>
                                        </div>
                                        <div
                                            className={
                                                styles.integrationDescription
                                            }
                                        >
                                            {integration.description}
                                        </div>
                                    </div>
                                ))}
                                {installedIntegrations.length === 0 && (
                                    <div className={styles.emptyState}>
                                        No integrations installed
                                    </div>
                                )}
                            </div>
                        </div>

                        {/* Available Integrations Section */}
                        <div className={styles.integrationSection}>
                            <h3 className={styles.sectionTitle}>
                                Available Integrations
                            </h3>
                            <div className={styles.integrationList}>
                                {availableIntegrations.map((integration) => (
                                    <Button
                                        key={integration.name}
                                        size="md"
                                        className={`${styles.integrationItem} ${
                                            selectedIntegration ===
                                            integration.name
                                                ? styles.selected
                                                : ""
                                        }`}
                                        onClick={() =>
                                            selectIntegration(integration.name)
                                        }
                                    >
                                        <div className={styles.integrationName}>
                                            {integration.name}
                                        </div>
                                        <div
                                            className={
                                                styles.integrationDescription
                                            }
                                        >
                                            {integration.description}
                                        </div>
                                    </Button>
                                ))}
                                {availableIntegrations.length === 0 && (
                                    <div className={styles.emptyState}>
                                        No new integrations available
                                    </div>
                                )}
                            </div>
                        </div>
                    </div>

                    <div className={styles.dialogButtons}>
                        <Button
                            variant="purple"
                            size="sm"
                            onClick={handleInstall}
                            disabled={
                                isLoading ||
                                isInstalling ||
                                !selectedIntegration
                            }
                        >
                            {isInstalling ? "Installing..." : "Add Integration"}
                        </Button>
                    </div>
                    <Dialog.Close asChild>
                        <div className={styles.closeButton}>
                            <Button
                                variant="ghost"
                                size="sm"
                                shape="square"
                                disabled={isLoading || isInstalling}
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
