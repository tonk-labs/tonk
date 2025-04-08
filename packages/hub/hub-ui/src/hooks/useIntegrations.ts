import { useCallback, useEffect, useState } from "react";
import { InstalledIntegration, Integration } from "../types";

interface UseIntegrationsReturn {
    integrations: Integration[];
    selectedIntegration: string | null;
    isLoading: boolean;
    error: string | null;
    selectIntegration: (name: string) => void;
    installedIntegrations: InstalledIntegration[];
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

    // Fetch both registry and installed integrations
    useEffect(() => {
        const fetchData = async () => {
            try {
                setIsLoading(true);
                setError(null);

                // Fetch registry and installed integrations in parallel
                const [registry, installed] = await Promise.all([
                    window.electronAPI.fetchRegistry(),
                    window.electronAPI.getInstalledIntegrations(),
                ]);

                if (!registry.success) {
                    throw new Error("Failed to fetch registry");
                }

                if (!installed.success) {
                    throw new Error("Failed to fetch installed integrations");
                }

                // Create a map of installed integrations for quick lookup
                const installedMap = new Map(
                    installed.data?.map((i) => [i.name, i]) || []
                );

                // Set installed integrations
                setInstalledIntegrations(installed.data || []);

                // Merge registry data with installed status
                setIntegrations(
                    registry.data.packages.map((integration) => ({
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
    }, []);

    const selectIntegration = useCallback((name: string) => {
        setSelectedIntegration(name);
    }, []);

    return {
        integrations,
        selectedIntegration,
        isLoading,
        error,
        selectIntegration,
        installedIntegrations,
    };
}
