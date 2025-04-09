import React from "react";
import styles from "./LaunchBar.module.css";
import { Button } from "../";
import { TreeItem, FileType } from "../Tree";
import { launchApp } from "../../ipc/app";
import { getConfig } from "../../ipc/config";
import { platformSensitiveJoin } from "../../ipc/files";
import { useProjectStore } from "../../stores/projectStore";

interface LaunchBarProps {
  commandCallback: (cmd: string) => void;
}

const AppLaunchBar = (
  selectedItem: TreeItem,
  commandCallback: (cmd: string) => void
) => {
  const launch = async () => {
    const config = await getConfig();
    const fullPath = await platformSensitiveJoin([
      config!.homePath,
      selectedItem.index,
    ]);
    launchApp(fullPath!);
  };

  const stopAndReset = () => {
    commandCallback("stopAndReset");
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
        Launch
      </Button>
    </div>
  );
};

const getComponentForItem = (
  selectedItem: TreeItem | null,
  commandCallback: (cmd: string) => void
) => {
  if (!selectedItem) {
    return null;
  }
  switch (selectedItem.data.fileType) {
    case FileType.App: {
      return AppLaunchBar(selectedItem, commandCallback);
    }
    default: {
      return null;
    }
  }
};

const LaunchBar: React.FC<LaunchBarProps> = ({ commandCallback }) => {
  const { selectedItem } = useProjectStore();
  const componentItem = getComponentForItem(selectedItem, commandCallback);
  return componentItem ? (
    <div className={styles.launchBarContainer}>{componentItem}</div>
  ) : null;
};

export default LaunchBar;
