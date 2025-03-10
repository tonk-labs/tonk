// @ts-nocheck
// Note: The ts-nocheck directive is added to temporarily bypass TypeScript errors
// while the proper type declarations aren't available

import React, { useState, useRef, useEffect } from "react";
import { Settings } from "lucide-react";
import styles from "./CLI.module.css";

// Import xterm.js and addons
import { Terminal } from "xterm";
import { FitAddon } from "xterm-addon-fit";
import { WebLinksAddon } from "xterm-addon-web-links";
import "xterm/css/xterm.css";

interface CLIProps {
  nav: (pageName: string) => void;
}

const CLI: React.FC<CLIProps> = (props: CLIProps) => {
  const [socket, setSocket] = useState<WebSocket | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [terminal, setTerminal] = useState<Terminal | null>(null);
  const [fitAddon, setFitAddon] = useState<FitAddon | null>(null);
  const terminalContainerRef = useRef<HTMLDivElement | null>(null);

  // Initialize xterm.js
  useEffect(() => {
    if (!terminalContainerRef.current) return;

    // Clear any existing terminal
    terminalContainerRef.current.innerHTML = "";

    // Create new terminal instance
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: '"Courier New", monospace',
      fontSize: 14,
      theme: {
        background: "#1e1e1e",
        foreground: "#f0f0f0",
        cursor: "#f0f0f0",
        selection: "rgba(170, 170, 170, 0.3)",
        black: "#000000",
        red: "#e06c75",
        green: "#98c379",
        yellow: "#e5c07b",
        blue: "#61afef",
        magenta: "#c678dd",
        cyan: "#56b6c2",
        white: "#dcdfe4",
        brightBlack: "#686868",
        brightRed: "#f74b4b",
        brightGreen: "#4bc94b",
        brightYellow: "#e5e510",
        brightBlue: "#6871ff",
        brightMagenta: "#ff77ff",
        brightCyan: "#5ffdff",
        brightWhite: "#ffffff",
      },
      disableStdin: false, // Ensure stdin isn't disabled
      convertEol: true, // Convert newlines to CRLF
    });

    // Create and register fit addon
    const fit = new FitAddon();
    term.loadAddon(fit);
    setFitAddon(fit);

    // Add web links addon (clickable links)
    term.loadAddon(new WebLinksAddon());

    // Open the terminal in the container
    term.open(terminalContainerRef.current);

    // Store the terminal instance
    setTerminal(term);

    // Fit the terminal to the container
    try {
      fit.fit();
    } catch (e) {
      console.error("Error fitting terminal:", e);
    }

    // Give the terminal focus to enable typing immediately
    setTimeout(() => {
      term.focus();
    }, 100);

    // Handle clicks to focus the terminal
    terminalContainerRef.current.addEventListener('click', () => {
      term.focus();
    });

    // Clean up on unmount
    return () => {
      term.dispose();
    };
  }, [terminalContainerRef]);
  
  // Add a separate effect to handle input data with access to the current socket
  useEffect(() => {
    if (!terminal) return;
    
    // Event listener for terminal input
    const dataListener = (data: string) => {
      if (socket && isConnected) {
        socket.send(
          JSON.stringify({
            type: "command",
            command: data,
          })
        );
      }
    };
    
    terminal.onData(dataListener);
    
    return () => {
      // Clean up by removing the listener
      // Note: xterm.js doesn't provide a direct way to remove listeners
      // but this will be handled by terminal.dispose() when component unmounts
    };
  }, [terminal, socket, isConnected]);

  // Set up WebSocket connection
  useEffect(() => {
    if (!terminal) return;

    // Connect to the WebSocket server
    const ws = new WebSocket("ws://localhost:3000");

    ws.onopen = () => {
      console.log("Connected to Terminal WebSocket server");
      setIsConnected(true);
      setSocket(ws);

      // Get terminal dimensions
      const { cols, rows } = terminal;

      // Send initial connection message to initialize the terminal session
      ws.send(
        JSON.stringify({
          type: "init",
          shell: "zsh",
          cols,
          rows,
        })
      );

      // Write welcome message (this will be replaced by server output)
      terminal.writeln("Connecting to terminal server...");
      
      // Ensure terminal has focus
      terminal.focus();
    };

    ws.onmessage = (event) => {
      try {
        let data;

        // Try to parse as JSON first
        try {
          data = JSON.parse(event.data);
        } catch (e) {
          // If it's not JSON, treat as raw text
          data = { type: "output", data: event.data };
        }

        console.log("Received terminal message:", data);

        // Handle different message types
        if (data.type === "output" && data.data) {
          terminal.write(data.data);
        } else if (data.type === "exit") {
          terminal.writeln(data.data);
        } else if (data.type === "error") {
          terminal.writeln(`\r\n\x1b[31m${data.data}\x1b[0m`); // Red error text
        } else if (data.output) {
          // Legacy support for old message format
          terminal.write(data.output);
        }
      } catch (error) {
        console.error("Error processing terminal message:", error);
        terminal.writeln(
          `\r\n\x1b[31mError processing message: ${error}\x1b[0m`
        );
      }
    };

    ws.onerror = (error) => {
      console.error("Terminal WebSocket error:", error);
      setIsConnected(false);
      terminal.writeln(`\r\n\x1b[31mWebSocket error: ${error}\x1b[0m`);
    };

    ws.onclose = () => {
      console.log("Disconnected from Terminal WebSocket server");
      setIsConnected(false);
      setSocket(null);
      terminal.writeln("\r\n\x1b[31mDisconnected from terminal server\x1b[0m");
    };

    return () => {
      ws.close();
    };
  }, [terminal]);

  // Handle window resize and terminal fitting
  useEffect(() => {
    if (!fitAddon || !terminal) return;

    const handleResize = () => {
      try {
        fitAddon.fit();

        // Send resize event to server if connected
        if (socket && isConnected) {
          const { cols, rows } = terminal;
          socket.send(
            JSON.stringify({
              type: "resize",
              cols,
              rows,
            })
          );
        }
      } catch (e) {
        console.error("Error resizing terminal:", e);
      }
    };

    // Initial fit
    handleResize();

    // Add event listener for window resize
    window.addEventListener("resize", handleResize);

    // Clean up
    return () => {
      window.removeEventListener("resize", handleResize);
    };
  }, [fitAddon, terminal, socket, isConnected]);

  return (
    <div className={styles.container}>
      <div
        className={styles.settingsButton}
        onClick={() => props.nav("Settings")}
      >
        <Settings size={20} />
      </div>

      <div 
        className={styles.terminalContainer} 
        ref={terminalContainerRef}
        tabIndex={1}
        onClick={() => terminal?.focus()}
      />

      {!isConnected && (
        <div className={styles.disconnectedAlert}>
          Disconnected from terminal server
        </div>
      )}
    </div>
  );
};

export default CLI;
