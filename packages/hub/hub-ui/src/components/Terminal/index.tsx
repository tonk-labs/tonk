import React, { useEffect, useRef, useState } from "react";
import styles from "./Terminal.module.css";
import { TerminalManager } from "./TerminalManager";

// Import xterm.js styles
import "xterm/css/xterm.css";
import { getConfig } from "../../ipc/config";
import { platformSensitiveJoin } from "../../ipc/files";
import { closeShell, runShell } from "../../ipc/hub";
import { useProjectStore } from "../../stores/projectStore";
import LaunchBar from "../LaunchBar";

const Terminal: React.FC = () => {
  const [isConnected, setIsConnected] = useState(false);
  const terminalContainerRef = useRef<HTMLDivElement | null>(null);
  const terminalManagerRef = useRef<TerminalManager | null>(null);
  const { selectedItem } = useProjectStore();
  const [path, setPath] = useState<string | null>(null);
  const [cmd, setCmd] = useState("");
  useEffect(() => {
    if (cmd !== "") {
      setTimeout(() => {
        setCmd("");
      }, 200);
    }
  }, [cmd]);

  useEffect(() => {
    const fn = async () => {
      if (selectedItem && path) {
        const config = await getConfig();
        const subPath = selectedItem.index.split("/");
        const fullPath = await platformSensitiveJoin([
          config!.homePath,
          ...subPath,
          path,
        ]);
        console.log("fullPath", fullPath);
        setPath(fullPath ?? null);
        closeShell().then(() => {
          runShell(fullPath!);
        });
      }
    };
    fn();
  }, [selectedItem, path]);
  // Initialize the terminal manager
  useEffect(() => {
    let mounted = true;

    const initializeTerminalManager = async () => {
      if (!terminalContainerRef.current || !mounted) return;

      // Create new terminal manager instance
      const manager = new TerminalManager({
        container: terminalContainerRef.current,
      });

      // Set connection change handler
      manager.setOnConnectionChange((connected) => {
        if (mounted) {
          setIsConnected(connected);
        }
      });

      // Initialize the terminal
      await manager.initialize();

      if (!mounted) {
        manager.dispose();
        return;
      }

      // Store the manager instance
      terminalManagerRef.current = manager;
    };

    // Wait for next tick to ensure container is mounted
    setTimeout(initializeTerminalManager, 0);

    // Clean up on unmount
    return () => {
      mounted = false;
      if (terminalManagerRef.current) {
        terminalManagerRef.current.dispose();
      }
    };
  }, []);

  // Connect to the selected item
  useEffect(() => {
    // wait for 50 ms before connecting to the selected item
    setTimeout(() => {
      if (!terminalManagerRef.current || !selectedItem) return;

      // Connect to the selected item
      terminalManagerRef.current.connectToItem(selectedItem);
    }, 50);
  }, [selectedItem]);

  // Handle commands
  useEffect(() => {
    if (cmd !== "" && terminalManagerRef.current) {
      terminalManagerRef.current.executeCommand(cmd);
    }
  }, [cmd]);

  return (
    <div className={styles.container}>
      <div
        key={1}
        className={styles.terminalContainer}
        ref={terminalContainerRef}
        tabIndex={1}
      />
      {!isConnected && (
        <div className={styles.disconnectedAlert}>
          Disconnected from terminal server
        </div>
      )}

      {/* The LaunchBar is placed at the bottom and will always be visible */}
      <div className={styles.launchBarContainer}>
        <LaunchBar commandCallback={setCmd} />
      </div>
    </div>
  );
};

export default Terminal;
