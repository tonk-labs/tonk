import React, { useState, useEffect } from "react";
import { useProjectStore } from "../../stores/projectStore";
import { readFile, platformSensitiveJoin } from "../../ipc/files";
import { closeShell, runShell } from "../../ipc/hub";
import { getConfig } from "../../ipc/config";
import styles from "./AppContent.module.css";
import Markdown from "react-markdown";
import Terminal from "../Terminal";

interface AppContentProps {}

const AppContent: React.FC<AppContentProps> = () => {
  const { selectedItem } = useProjectStore();
  const [fileContent, setFileContent] = useState("");

  useEffect(() => {
    const fn = async () => {
      if (selectedItem) {
        const config = await getConfig();
        const subPath = selectedItem.index.split("/");
        const fullPath = await platformSensitiveJoin([
          config!.homePath,
          ...subPath,
        ]);
        // const content = await readFile(fullPath!);
        closeShell().then(() => {
          runShell(fullPath!);
        });
        // setFileContent(content!);
      }
    };
    fn();
  }, [selectedItem]);

  return <Terminal />;
};

export default AppContent;
