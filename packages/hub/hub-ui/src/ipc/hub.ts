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

export const fetchRegistry = async () => {
  await window.electronAPI.fetchRegistry();
};

export const runShell = async (dirPath: string) => {
  await window.electronAPI.runShell(dirPath);
};

export const closeShell = async () => {
  await window.electronAPI.closeShell();
};

export const createApp = async (name: string) => {
  await window.electronAPI.createApp(name);
};
