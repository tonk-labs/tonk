export const handleSelectDirectory = async (
  onSelected: (path: string) => void
) => {
  try {
    const result = await window.electronAPI.showOpenDialog({
      properties: ["openDirectory"],
    });

    if (!result.canceled) {
      const path = result.filePaths[0];
      onSelected(path);
    }
  } catch (error: unknown) {
    console.error("Error selecting folder:", error);
    if (error instanceof Error) {
      alert("Error selecting folder: " + error.message);
    } else {
      alert("An unexpected error occurred while selecting folder");
    }
  }
};
