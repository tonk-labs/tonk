import { useCallback, useEffect, useState } from "react";
import { InstalledIntegration, Integration } from "../types";

interface UseIntegrationsReturn {
    integrations: Integration[];
    selectedIntegration: string | null;
    isLoading: boolean;
    error: string | null;
    selectIntegration: (name: string) => void;
    installedIntegrations: InstalledIntegration[];
    installIntegration: () => Promise<void>;
}

export function useIntegrations(): UseIntegrationsReturn {
    const [integrations, setIntegrations] = useState<Integration[]>([]);
    const [selectedIntegration, setSelectedIntegration] = useState<
        string | null
    >(null);
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [installedIntegrations, setInstalledIntegrations] = useState<
        InstalledIntegration[]
    >([]);

    const fetchRegistry = useCallback(async () => {
        const registry = await window.electronAPI.fetchRegistry();
        if (!registry.success) {
            throw new Error("Failed to fetch registry");
        }
        return registry.data.packages;
    }, []);

    const fetchInstalledIntegrations = useCallback(async () => {
        const installed = await window.electronAPI.getInstalledIntegrations();
        if (!installed.success) {
            throw new Error("Failed to fetch installed integrations");
        }
        setInstalledIntegrations(installed.data || []);
        return installed.data || [];
    }, []);

    // Fetch both registry and installed integrations
    useEffect(() => {
        const fetchData = async () => {
            try {
                setIsLoading(true);
                setError(null);

                // Execute both fetch callbacks in parallel
                const [registryPackages, installedData] = await Promise.all([
                    fetchRegistry(),
                    fetchInstalledIntegrations(),
                ]);

                // Create a map of installed integrations for quick lookup
                const installedMap = new Map(
                    installedData.map((i) => [i.name, i])
                );

                // Merge registry data with installed status
                setIntegrations(
                    registryPackages.map((integration) => ({
                        name: integration.name,
                        link: integration.link,
                        description: integration.description,
                        isInstalled: installedMap.has(integration.name),
                        version: installedMap.get(integration.name)?.version,
                    }))
                );
            } catch (err) {
                setError(
                    err instanceof Error
                        ? err.message
                        : "Failed to fetch integrations"
                );
                console.error("Error fetching integrations:", err);
            } finally {
                setIsLoading(false);
            }
        };

        fetchData();
    }, [fetchRegistry, fetchInstalledIntegrations]);

    const selectIntegration = useCallback((name: string) => {
        setSelectedIntegration(name);
    }, []);

    const installIntegration = useCallback(async () => {
        if (!selectedIntegration) return;

        try {
            setIsLoading(true);
            const link = integrations.find(
                (integration) => integration.name === selectedIntegration
            )?.link;
            if (!link) {
                throw new Error("Integration link not found");
            }
            const result =
                await window.electronAPI.installIntegration(link);
            if (!result.success) {
                throw new Error(result.error);
            }
            await fetchInstalledIntegrations();
        } catch (err) {
            console.error("Failed to install integration:", err);
        } finally {
            setIsLoading(false);
        }
    }, [selectedIntegration, integrations]);
    return {
        integrations,
        selectedIntegration,
        isLoading,
        error,
        selectIntegration,
        installedIntegrations,
        installIntegration,
    };
}
