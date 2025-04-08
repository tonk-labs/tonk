export const launchApp = async (appPath: string) => {
  try {
    // Launch the app with the selected docId
    await window.electronAPI.launchApp(appPath);
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
    await window.electronAPI.openExternal(link);
  } catch (error: unknown) {
    console.error("Error launching app:", error);
    if (error instanceof Error) {
      alert("Error launching app: " + error.message);
    } else {
      alert("An unexpected error occurred while launching the app");
    }
  }
};
