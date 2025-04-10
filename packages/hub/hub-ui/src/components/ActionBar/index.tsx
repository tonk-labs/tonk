import * as Dialog from "@radix-ui/react-dialog";
import { BookOpenText, HomeIcon, NotebookPen } from "lucide-react";
import React, { useState } from "react";
import { openExternal } from "../../ipc/app";
import { clearConfig, createApp } from "../../ipc/hub";
import { useEventStore } from "../../stores/eventStore";
import { useProjectStore } from "../../stores/projectStore";
import Button from "../Button";
import styles from "./ActionBar.module.css";
import CreateAppDialog from "./CreateAppDialog";

const ActionBar: React.FC = () => {
  const [isOpen, setIsOpen] = useState(false);

  const { setSelectedItem } = useProjectStore();

  const openDocs = () => {
    openExternal("https://tonk-labs.github.io/tonk/");
  };

  

  const handleGoHome = () => {
    setSelectedItem(null);
  };



  return (
    <div className={styles.actionBar}>
      <Button
        size="sm"
        shape="square"
        color="ghost"
        onClick={handleGoHome}
        tooltip="Home"
        tooltipPosition="bottom"
      >
        <HomeIcon size={24} />
      </Button>
      <Dialog.Root open={isOpen} onOpenChange={setIsOpen}>
        <Dialog.Trigger asChild>
          <Button
            size="sm"
            shape="square"
            color="ghost"
            title="Create App"
            tooltip="Create App"
            tooltipPosition="bottom"
          >
            <NotebookPen size={24} />
          </Button>
        </Dialog.Trigger>
        <CreateAppDialog
          close={() => setIsOpen(false)}
        />
      </Dialog.Root>
      <Button
        color="ghost"
        size="sm"
        shape="square"
        title="Documentation"
        onClick={openDocs}
        tooltip="Documentation"
        tooltipPosition="bottom"
      >
        <BookOpenText size={24} />
      </Button>
    </div>
  );
};

export default ActionBar;
