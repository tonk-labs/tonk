import { useCallback, useState } from "react";

import { useEffect } from "react";

export const useApp = () => {
    const [isAppRunning, setIsAppRunning] = useState(false);
    const checkAppRunning = useCallback(async () => {
        try {
            const isAppRunning = await window.electronAPI.isAppRunning();
            console.log("isAppRunning", isAppRunning);
            setIsAppRunning(isAppRunning);
        } catch (error) {
            console.error("Error checking app status:", error);
            setIsAppRunning(false);
        }
    }, []);

    useEffect(() => {
        checkAppRunning();
    }, []);

    return { isAppRunning, checkAppRunning };
};
