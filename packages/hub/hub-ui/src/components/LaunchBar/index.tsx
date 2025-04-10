import { useState } from "react";
import { Button } from "../";
import { launchApp } from "../../ipc/app";
import { getConfig } from "../../ipc/config";
import { platformSensitiveJoin } from "../../ipc/files";
import { useProjectStore } from "../../stores/projectStore";
import { FileType, TreeItem } from "../Tree";
import styles from "./LaunchBar.module.css";

const AppLaunchBar = (props: {
  selectedItem: TreeItem;
  commandCallback: (cmd: string) => void;
}) => {
  const { selectedItem, commandCallback } = props;
  const [url, setUrl] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const launch = async () => {
    try {
      const config = await getConfig();
      const fullPath = await platformSensitiveJoin([
        config!.homePath,
        selectedItem.index,
      ]);
      const url = await launchApp(fullPath!);
      setUrl(url || null);
    } catch (error) {
      console.error("Error launching app:", error);
      setUrl(null);
    }
  };

  const copyToClipboard = () => {
    if (url) {
      navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <div className={styles.buttonArea}>
      <Button color="green" size="sm" shape="rounded" onClick={launch}>
        Launch
      </Button>
      {url && (
        <div
          style={{
            fontSize: "12px",
            opacity: 0.8,
            display: "flex",
            alignItems: "center",
            gap: "5px",
            marginTop: "5px",
          }}
        >
          Your app is live at{" "}
          <Button size="sm" onClick={copyToClipboard} title="Click to copy URL">
            <span style={{ marginRight: "5px" }}>{url}</span>
            <span style={{ fontSize: "14px" }}>{copied ? "✓" : "📋"}</span>
          </Button>
        </div>
      )}
    </div>
  );
};

const LaunchBar = (props: { commandCallback: (cmd: string) => void }) => {
  const { selectedItem } = useProjectStore();
  const { commandCallback } = props;
  if (!selectedItem) {
    return null;
  }
  switch (selectedItem.data.fileType) {
    case FileType.App: {
      return (
        <AppLaunchBar
          selectedItem={selectedItem}
          commandCallback={commandCallback}
        />
      );
    }
    default: {
      return null;
    }
  }
};

export default LaunchBar;
