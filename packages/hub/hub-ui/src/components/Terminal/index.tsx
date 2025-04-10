import React, { useEffect, useRef, useState } from "react";
import { FileType } from "../Tree";
import styles from "./Terminal.module.css";
import { TerminalManager } from "./TerminalManager";

// Import xterm.js styles
import "xterm/css/xterm.css";

interface TerminalProps {
  selectedItem: {
    data: {
      fileType: FileType;
      name: string;
    };
  } | null;
  cmd: string;
}

const Terminal: React.FC<TerminalProps> = ({ selectedItem, cmd }) => {
  const [isConnected, setIsConnected] = useState(false);
  const terminalContainerRef = useRef<HTMLDivElement | null>(null);
  const terminalManagerRef = useRef<TerminalManager | null>(null);

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
    </div>
  );
};

export default Terminal;
