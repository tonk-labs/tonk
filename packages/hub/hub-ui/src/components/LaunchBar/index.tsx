import React from "react";
import styles from "./LaunchBar.module.css";
import { Button } from "../";
import { TreeItem, FileType } from "../Tree";
import { openExternal } from "../../ipc/app";
import { useProjectStore } from "../../stores/projectStore";

interface LaunchBarProps {}

const AppLaunchBar = () => {
  return (
    <div className={styles.buttonArea}>
      <Button variant={"green"}>Launch</Button>
    </div>
  );
};

const getComponentForItem = (selectedItem: TreeItem | null) => {
  if (!selectedItem) {
    return null;
  }
  switch (selectedItem.data.fileType) {
    case FileType.App: {
      return AppLaunchBar();
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
