import React, { useState, useEffect } from "react";
import { useProjectStore } from "../../stores/projectStore";
import { readFile, platformSensitiveJoin } from "../../ipc/files";
import { getConfig } from "../../ipc/config";
import styles from "./AppContent.module.css";
import Markdown from "react-markdown";

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
          "README.md",
        ]);
        const content = await readFile(fullPath!);
        setFileContent(content!);
      }
    };
    fn();
  }, [selectedItem]);

  return <Markdown>{fileContent}</Markdown>;
};

export default AppContent;
