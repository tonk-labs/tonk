import React from "react";
import styles from "./LaunchBar.module.css";
import { Button } from "../";
import { TreeItem, FileType } from "../Tree";
import { launchApp } from "../../ipc/app";
import { getConfig } from "../../ipc/config";
import { platformSensitiveJoin } from "../../ipc/files";
import { useProjectStore } from "../../stores/projectStore";

interface LaunchBarProps {}

const AppLaunchBar = (selectedItem: TreeItem) => {
  const launch = async () => {
    const config = await getConfig();
    const fullPath = await platformSensitiveJoin([
      config!.homePath,
      selectedItem.index,
    ]);
    launchApp(fullPath!);
  };
  return (
    <div className={styles.buttonArea}>
      <Button variant={"green"} onClick={launch}>
        Launch
      </Button>
    </div>
  );
};

const getComponentForItem = (selectedItem: TreeItem | null) => {
  if (!selectedItem) {
    return null;
  }
  switch (selectedItem.data.fileType) {
    case FileType.App: {
      return AppLaunchBar(selectedItem);
    }
    default: {
      return null;
    }
  }
};

const LaunchBar: React.FC<LaunchBarProps> = () => {
  const { selectedItem } = useProjectStore();
  const componentItem = getComponentForItem(selectedItem);
  return componentItem ? (
    <div className={styles.launchBarContainer}>{componentItem}</div>
  ) : null;
};

export default LaunchBar;
