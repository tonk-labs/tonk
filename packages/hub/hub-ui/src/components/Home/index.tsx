import React, { useEffect, useState } from "react";
import {
  FileViewer,
  LaunchBar,
  Link,
  LinkType,
  Text,
  TonkAsciiAnimated,
} from "..";

import * as Dialog from "@radix-ui/react-dialog";
import { openExternal } from "../../ipc/app";
import { useProjectStore } from "../../stores/projectStore";
import CreateAppDialog from "../ActionBar/CreateAppDialog";
import AppContent from "../AppContent";
import { FileType } from "../Tree";
import styles from "./Home.module.css";

const EmptyState = () => {
  const [isOpen, setIsOpen] = useState(false);
  const openGuide = () => {
    openExternal("https://tonk.xyz");
  };

  const { items, setSelectedItem } = useProjectStore();

  const handleSelectApp = async (appName: string) => {
    const app = items[appName];
    if (!app) {
      return;
    }
    try {
      setSelectedItem(app);
    } catch (error) {
      console.error("Error launching app:", error);
    }
  };

  return (
    <>
      <TonkAsciiAnimated key={0} />
      <Text style={{ fontSize: "12pt" }}>Welcome to your Tonk Hub!</Text>

      {items.apps && items.apps.children.length > 0 ? (
        <>
          <Text>&nbsp;</Text>
          <Text>Your Apps:</Text>
          <div>
            <ul className={styles.appsList}>
              {items.apps.children.map((appName, index) => (
                <li
                  key={index}
                  className={styles.appItem}
                  onClick={() => handleSelectApp(appName)}
                >
                  <Text>{appName.slice(5)}</Text>
                </li>
              ))}
  <Dialog.Root open={isOpen} onOpenChange={setIsOpen}>
              <Dialog.Trigger asChild>
                <div className={styles.appItem} style = {{marginLeft: "-16px", color: "#35ff3c"}}>
                  <Text style = {{color: "blue"}}>+ Create new app</Text>
                </div>
              </Dialog.Trigger>
              <CreateAppDialog close={() => setIsOpen(false)} />
            </Dialog.Root>
            </ul>
          
          </div>
        </>
      ) : (
        <>
          <Text>&nbsp;</Text>
          <Text>
            Looks like this might be your first time.&nbsp;
            <Link onClick={openGuide} linkType={LinkType.External}>
              Check out our getting started guide.
            </Link>
          </Text>
        </>
      )}
    </>
  );
};

const Content = (props: { cmd: string }) => {
  const { cmd } = props;
  const { selectedItem } = useProjectStore();
  if (!selectedItem) {
    return <EmptyState />;
  }
  switch (selectedItem.data.fileType) {
    case FileType.App: {
      return <AppContent cmd={cmd} />;
    }
    case FileType.Store: {
      return <FileViewer />;
    }
    default: {
      return <EmptyState />;
    }
  }
};

const Home: React.FC = () => {
  const [cmd, setCmd] = useState("");
  useEffect(() => {
    if (cmd !== "") {
      setTimeout(() => {
        setCmd("");
      }, 200);
    }
  }, [cmd]);
  return (
    <div className={styles.contentArea}>
      <Content cmd={cmd} />
      <LaunchBar commandCallback={setCmd} />
    </div>
  );
};

export default Home;
