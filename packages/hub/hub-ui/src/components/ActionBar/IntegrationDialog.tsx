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
        installIntegration,
    } = useIntegrations();

    // Filter available integrations to only show uninstalled ones
    const availableIntegrations = integrations.filter(
        (integration) => !integration.isInstalled
    );

    return (
        <Dialog.Root open={isOpen} onOpenChange={setIsOpen}>
            <Dialog.Trigger asChild>
                <Button
                    color="ghost"
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
                                        className={`${styles.integrationItem}`}
                                        disabled={
                                            installedIntegrations.some(
                                                (installedIntegration) =>
                                                    installedIntegration.name ===
                                                    integration.name
                                            )
                                        }
                                        style={{
                                            border:
                                                selectedIntegration ===
                                                integration.name
                                                    ? "1px solid #007bb5"
                                                    : "1px solid transparent",
                                        }}
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
                            color="green"
                            size="sm"
                            onClick={installIntegration}
                            disabled={isLoading || !selectedIntegration}
                        >
                            {isLoading ? "Installing..." : "Add Integration"}
                        </Button>
                    </div>
                    <Dialog.Close asChild>
                        <Button
                            color="ghost"
                            size="sm"
                            shape="square"
                            disabled={isLoading}
                            aria-label="Close"
                            className={styles.closeButton}
                        >
                            ×
                        </Button>
                    </Dialog.Close>
                </Dialog.Content>
            </Dialog.Portal>
        </Dialog.Root>
    );
};

export default IntegrationDialog;
