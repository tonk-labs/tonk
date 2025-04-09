import React from "react";
import styles from "./LaunchBar.module.css";
import { Button } from "../";
import { TreeItem, FileType } from "../Tree";
import { launchApp } from "../../ipc/app";
import { getConfig } from "../../ipc/config";
import { platformSensitiveJoin } from "../../ipc/files";
import { useProjectStore } from "../../stores/projectStore";
import { useApp } from "../../hooks/useApp";

const AppLaunchBar = (
  props: {
    selectedItem: TreeItem,
    commandCallback: (cmd: string) => void
  }
) => {
  const { selectedItem, commandCallback } = props;
  const { isAppRunning, checkAppRunning } = useApp();
  const launch = async () => {
    const config = await getConfig();
    const fullPath = await platformSensitiveJoin([
      config!.homePath,
      selectedItem.index,
    ]);
    await launchApp(fullPath!);
    await checkAppRunning();
  };

  const stopAndReset = async () => {
    try {
      await window.electronAPI.stopAndReset();
      commandCallback("stopAndReset");
    } catch (error) {
      console.error(error);
    }
  };

  return (
    <div className={styles.buttonArea}>
      <Button
        size="sm"
        shape="rounded"
        onClick={stopAndReset}
      >
        Stop & Reset
      </Button>
      <Button color="green" size="sm" shape="rounded" onClick={launch}>
        {isAppRunning ? "Open" : "Launch"}
      </Button>
    </div>
  );
};

const LaunchBar = (
  props: {
    commandCallback: (cmd: string) => void
  }
) => {
  const { selectedItem } = useProjectStore();
  const { commandCallback } = props;
  if (!selectedItem) {
    return null;
  }
  switch (selectedItem.data.fileType) {
    case FileType.App: {
      return <AppLaunchBar selectedItem={selectedItem} commandCallback={commandCallback} />;
    }
    default: {
      return null;
    }
  }
};

export default LaunchBar;
