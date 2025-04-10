import { useCallback, useEffect, useRef, useState } from "react";
import {
  FileViewer,
  Link,
  LinkType,
  Terminal,
  Text,
  TonkAsciiAnimated,
} from "..";

import * as Dialog from "@radix-ui/react-dialog";
import { openExternal } from "../../ipc/app";
import { useProjectStore } from "../../stores/projectStore";
import CreateAppDialog from "../ActionBar/CreateAppDialog";
import { FileType } from "../Tree";
import styles from "./Home.module.css";

const EmptyState = () => {
  const [isOpen, setIsOpen] = useState(false);
  const [focusedIndex, setFocusedIndex] = useState<number>(-1);
  const appItemsRef = useRef<HTMLLIElement[]>([]);
  const createNewRef = useRef<HTMLDivElement>(null);
  const openGuide = () => {
    openExternal("https://tonk.xyz");
  };

  const { items, setSelectedItem } = useProjectStore();
  const appsList = items.apps?.children || [];
  const totalItems = appsList.length + 1; // +1 for the create new app button

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

  const handleKeyNavigation = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "j" || e.key === "ArrowDown") {
        // Move down
        setFocusedIndex((prevIndex) => {
          const newIndex = prevIndex >= totalItems - 1 ? 0 : prevIndex + 1;
          return newIndex;
        });
      } else if (e.key === "k" || e.key === "ArrowUp") {
        // Move up
        setFocusedIndex((prevIndex) => {
          const newIndex = prevIndex <= 0 ? totalItems - 1 : prevIndex - 1;
          return newIndex;
        });
      } else if (e.key === "Enter" && focusedIndex !== -1) {
        if (focusedIndex < appsList.length) {
          handleSelectApp(appsList[focusedIndex]);
        } else {
          setIsOpen(true);
        }
      }
    },
    [totalItems, appsList, focusedIndex]
  );

  useEffect(() => {
    window.addEventListener("keydown", handleKeyNavigation);
    return () => {
      window.removeEventListener("keydown", handleKeyNavigation);
    };
  }, [handleKeyNavigation]);

  useEffect(() => {
    // Focus the appropriate element when focusedIndex changes
    if (focusedIndex >= 0) {
      if (focusedIndex < appsList.length) {
        appItemsRef.current[focusedIndex]?.focus();
      } else {
        createNewRef.current?.focus();
      }
    }
  }, [focusedIndex, appsList.length]);

  return (
    <div
      style={{
        padding: 10,
        width: "100%",
        display: "flex",
        flexDirection: "column",
        gap: "14px",
      }}
    >
      <TonkAsciiAnimated key={0} />
      <Text style={{ fontSize: "12pt" }}>Welcome to your Tonk Hub!</Text>
      <Link onClick={openGuide} linkType={LinkType.External}>
        Check out our getting started guide.
      </Link>
      {!!items.apps && items.apps.children.length > 0 && (
        <div
          style={{
            border: "1px solid white",
            marginTop: "10px",
            padding: "10px",
            width: "fit-content",
          }}
        >
          <Text>Your Apps:</Text>
          <ul className={styles.appsList}>
            {items.apps.children.map((appName, index) => (
              <li
                key={index}
                className={styles.appItem}
                onClick={() => handleSelectApp(appName)}
                onFocus={() => setFocusedIndex(index)}
                ref={(el) => el && (appItemsRef.current[index] = el)}
                tabIndex={0}
                style={
                  focusedIndex === index
                    ? { backgroundColor: "rgba(255, 255, 255, 0.1)" }
                    : {}
                }
              >
                <Text>{appName.slice(5)}</Text>
              </li>
            ))}
            <Dialog.Root open={isOpen} onOpenChange={setIsOpen}>
              <Dialog.Trigger asChild>
                <div
                  className={styles.appItem}
                  style={{
                    marginLeft: "-16px",
                    color: "#35ff3c",
                    backgroundColor:
                      focusedIndex === appsList.length
                        ? "rgba(255, 255, 255, 0.1)"
                        : undefined,
                  }}
                  onClick={() => setIsOpen(true)}
                  onFocus={() => setFocusedIndex(appsList.length)}
                  ref={createNewRef}
                  tabIndex={0}
                >
                  <Text style={{ color: "blue" }}>+ Create new app</Text>
                </div>
              </Dialog.Trigger>
              <CreateAppDialog close={() => setIsOpen(false)} />
            </Dialog.Root>
          </ul>
        </div>
      )}
    </div>
  );
};

const Home = () => {
  const { selectedItem } = useProjectStore();
  if (!selectedItem) {
    return <EmptyState />;
  }
  switch (selectedItem.data.fileType) {
    case FileType.App: {
      return <Terminal />;
    }
    case FileType.Store: {
      return <FileViewer />;
    }
    default: {
      return <EmptyState />;
    }
  }
};

export default Home;
