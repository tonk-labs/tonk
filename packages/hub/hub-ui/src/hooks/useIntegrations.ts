import { useCallback, useEffect, useState } from "react";
import { Registry } from "../types";

interface Integration {
    name: string;
    isInstalled: boolean;
}

interface UseIntegrationsReturn {
    integrations: Integration[];
    selectedIntegration: string | null;
    isLoading: boolean;
    error: string | null;
    selectIntegration: (name: string) => void;
}

export function useIntegrations(): UseIntegrationsReturn {
    const [integrations, setIntegrations] = useState<Integration[]>([]);
    const [selectedIntegration, setSelectedIntegration] = useState<
        string | null
    >(null);
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        const fetchIntegrations = async () => {
            try {
                setIsLoading(true);
                setError(null);

                const registry = await window.electronAPI.fetchRegistry();

                if (!registry.success) {
                    throw new Error("Failed to fetch registry");
                }

                setIntegrations(
                    registry.data.packages.map((name) => ({
                        name,
                        isInstalled: false,
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

        fetchIntegrations();
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
    };
}
