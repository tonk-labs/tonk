export const handleSelectDirectory = async () => {
  const result = await window.electronAPI.showOpenDialog({
    properties: ["openDirectory"],
  });

  if (result.canceled) {
    throw new Error("User canceled the dialog");
  }

  const path = result.filePaths[0];
  if (!path) {
    throw new Error("No path selected");
  }
  return path;
};
