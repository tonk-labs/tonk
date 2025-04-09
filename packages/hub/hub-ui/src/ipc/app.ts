export const launchApp = async (appPath: string) => {
    try {
        // Launch the app with the selected docId
        const url = await window.electronAPI.launchApp(appPath);
        return url;
    } catch (error: unknown) {
        console.error("Error launching app:", error);
        if (error instanceof Error) {
            alert("Error launching app: " + error.message);
        } else {
            alert("An unexpected error occurred while launching the app");
        }
    }
};


export const openExternal = async (link: string) => {
    try {
        // Launch the app with the selected docId
        return await window.electronAPI.openExternal(link);
    } catch (error: unknown) {
        console.error("Error launching app:", error);
        if (error instanceof Error) {
            alert("Error launching app: " + error.message);
        } else {
            alert("An unexpected error occurred while launching the app");
        }
    }
};
