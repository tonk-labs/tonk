export const getConfig = async () => {
  try {
    // Launch the app with the selected docId
    return await window.electronAPI.getConfig();
  } catch (error: unknown) {
    console.error("Error fetching config:", error);
    if (error instanceof Error) {
      console.error("Error fetching config: " + error.message);
    } else {
      console.error("An unexpected error occurred while launching the app");
    }
  }
};
