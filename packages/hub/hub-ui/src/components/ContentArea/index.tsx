import React, { useState, useEffect } from "react";
import styles from "./ContentArea.module.css";
import {
  Text,
  TonkAsciiAnimated,
  Link,
  LinkType,
  LaunchBar,
  FileViewer,
} from "..";
import { TreeItem, FileType } from "../Tree";
import { openExternal } from "../../ipc/app";
import { useProjectStore } from "../../stores/projectStore";
import AppContent from "../AppContent";

interface ContentAreaProps {}

const EmptyState = () => {
  const openGuide = () => {
    openExternal("https://tonk.xyz");
  };

  return (
    <>
      <TonkAsciiAnimated key={0} />
      <Text>Welcome to your Tonk Home!</Text>
      <Text>&nbsp;</Text>
      <Text>
      Looks like this might be your first time.&nbsp;
      <Link onClick={openGuide} linkType={LinkType.External}>
          Check out our getting started guide.
        </Link>
      </Text>
    </>
  );
};

const Content = (props: { selectedItem: TreeItem | null; cmd: string }) => {
  const { selectedItem, cmd } = props;
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

const ContentArea: React.FC<ContentAreaProps> = () => {
  const { selectedItem } = useProjectStore();
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
      <Content selectedItem={selectedItem} cmd={cmd} />
      <LaunchBar commandCallback={setCmd} />
    </div>
  );
};

export default ContentArea;
