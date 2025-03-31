export const init = async (homePath: string) => {
  if (!homePath) {
    alert("Please select both a folder and enter a document ID");
    return;
  }
  // Launch the app with the selected docId
  await window.electronAPI.init(homePath);
};

export const copyHubTemplate = async () => {
  await window.electronAPI.copyHubTemplate();
};
