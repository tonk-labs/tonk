import React, { useEffect, useState } from "react";
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
  const [path, setPath] = useState<string | null>(null);

  useEffect(() => {
    const fn = async () => {

      if (selectedItem) {
        const config = await getConfig();
        const subPath = selectedItem.index.split("/");
        const fullPath = await platformSensitiveJoin([
          config!.homePath,
          ...subPath,
        ]);
        setPath(fullPath ?? null);
        closeShell().then(() => {
          runShell(fullPath!);
        });
      }
    };
    fn();
  }, [selectedItem]);

  if (!path) {
    return <div>Loading...</div>;
  }
  return <Terminal selectedItem={selectedItem} cmd={cmd} path={path} />;
};

export default AppContent;
