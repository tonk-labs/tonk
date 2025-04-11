import { getConfig } from "./config";
import { platformSensitiveJoin } from "./files";

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

export const launchAppDev = async (appPath: string) => {
  try {
    // Launch the app with the selected docId
    const url = await window.electronAPI.launchAppDev(appPath);
    return url;
  } catch (error: unknown) {
    console.error("Error launching app in dev mode:", error);
    if (error instanceof Error) {
      alert("Error launching app in dev mode: " + error.message);
    } else {
      alert("An unexpected error occurred while launching the app in dev mode");
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

export const getApps = async () => {
  const config = await getConfig();
  const fullPath = await platformSensitiveJoin([config!.homePath, "apps"]);
  if (!fullPath) {
    return [];
  }
  const apps = await window.electronAPI.ls(fullPath);
  return apps.map((app) => app.name);
};

