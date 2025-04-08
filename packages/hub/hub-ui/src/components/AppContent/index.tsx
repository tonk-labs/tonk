import React, { useEffect } from "react";
import { useProjectStore } from "../../stores/projectStore";
import { closeShell, runShell } from "../../ipc/hub";
import { getConfig } from "../../ipc/config";
import { platformSensitiveJoin } from "../../ipc/files";
import Terminal from "../Terminal";

interface AppContentProps {
  cmd: string;
}

const AppContent: React.FC<AppContentProps> = ({ cmd }) => {
  const { selectedItem } = useProjectStore();

  useEffect(() => {
    const fn = async () => {
      if (selectedItem) {
        const config = await getConfig();
        const subPath = selectedItem.index.split("/");
        const fullPath = await platformSensitiveJoin([
          config!.homePath,
          ...subPath,
        ]);

        closeShell().then(() => {
          runShell(fullPath!);
        });
      }
    };
    fn();
  }, [selectedItem]);

  return <Terminal selectedItem={selectedItem} cmd={cmd} />;
};

export default AppContent;
