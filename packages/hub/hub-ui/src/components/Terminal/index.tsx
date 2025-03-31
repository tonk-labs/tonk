// @ts-nocheck
// Note: The ts-nocheck directive is added to temporarily bypass TypeScript errors
// while the proper type declarations aren't available

import React, { useState, useRef, useEffect } from "react";
import styles from "./Terminal.module.css";

// Import xterm.js and addons
import { Terminal as XTerm } from "xterm";
import { FitAddon } from "xterm-addon-fit";
import { WebLinksAddon } from "xterm-addon-web-links";
import "xterm/css/xterm.css";

interface TerminalProps {}

const Terminal: React.FC<TerminalProps> = (props: TerminalProps) => {
  const [socket, setSocket] = useState<WebSocket | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [terminal, setTerminal] = useState<XTerm | null>(null);
  const [fitAddon, setFitAddon] = useState<FitAddon | null>(null);
  const terminalContainerRef = useRef<HTMLDivElement | null>(null);
  const [isTerminalReady, setIsTerminalReady] = useState(false);

  // Initialize xterm.js
  useEffect(() => {
    let term: XTerm | null = null;
    let fit: FitAddon | null = null;
    let mounted = true;

    const initializeTerminal = async () => {
      if (!terminalContainerRef.current || !mounted) return;

      // Clear any existing terminal
      terminalContainerRef.current.innerHTML = "";

      // Create new terminal instance
      term = new XTerm({
        cursorBlink: true,
        fontFamily: '"Courier New", monospace',
        fontSize: 14,
        rows: 24,
        cols: 80,
        theme: {
          background: "rgba(30, 30, 30, 0.8)", // Semi-transparent background
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
        allowProposedApi: true,
        convertEol: true,
        disableStdin: false,
        rendererType: "dom", // Use DOM renderer for better Electron compatibility
        allowTransparency: true, // Enable transparency support
      });

      // Create and register fit addon
      fit = new FitAddon();
      term.loadAddon(fit);

      // Add web links addon
      term.loadAddon(new WebLinksAddon());

      // Open the terminal in the container
      term.open(terminalContainerRef.current);

      // Wait for the terminal to be ready
      await new Promise((resolve) => {
        const checkReady = () => {
          if (term?.element && terminalContainerRef.current?.offsetHeight > 0) {
            resolve(true);
          } else {
            setTimeout(checkReady, 10);
          }
        };
        checkReady();
      });

      if (!mounted) {
        term.dispose();
        return;
      }

      // Store the terminal instance and fit addon
      setTerminal(term);
      setFitAddon(fit);
      setIsTerminalReady(true);

      // Initial fit
      try {
        fit.fit();
        term.focus();
      } catch (e) {
        console.error("Error during initial terminal fit:", e);
      }
    };

    // Wait for next tick to ensure container is mounted
    setTimeout(initializeTerminal, 0);

    // Clean up on unmount
    return () => {
      mounted = false;
      if (term) {
        term.dispose();
      }
    };
  }, []);

  // Handle terminal input
  useEffect(() => {
    if (!terminal || !isTerminalReady) return;

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
  }, [terminal, socket, isConnected, isTerminalReady]);

  // Set up WebSocket connection
  useEffect(() => {
    if (!terminal || !fitAddon || !isTerminalReady) return;

    let retryCount = 0;
    const maxRetries = 3;
    let retryTimeout: NodeJS.Timeout;
    let lastRetryAttempt = 0;
    const retryDebounceWindow = 2000; // 2 seconds window between retries

    const scheduleRetry = () => {
      const now = Date.now();
      if (now - lastRetryAttempt < retryDebounceWindow) {
        console.log("Skipping retry - too soon since last attempt");
        return;
      }

      if (retryCount < maxRetries) {
        retryCount++;
        lastRetryAttempt = now;
        terminal.writeln(
          `\r\n\x1b[33mRetrying connection in 1 second... (${retryCount}/${maxRetries})\x1b[0m`
        );
        clearTimeout(retryTimeout);
        retryTimeout = setTimeout(connectWebSocket, 1000);
      } else {
        terminal.writeln(
          "\r\n\x1b[31mMax retries exceeded. Please check if the terminal server is running.\x1b[0m"
        );
      }
    };

    const connectWebSocket = () => {
      // Get the WebSocket URL based on the environment
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const hostname = window.location.hostname || "localhost";
      const wsUrl = `${protocol}//${hostname}:3060`;

      console.log(
        `Connecting to WebSocket server at ${wsUrl} (attempt ${retryCount + 1}/${maxRetries})`
      );
      const ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        if (!terminal.element) return;

        console.log("Connected to Terminal WebSocket server");
        setIsConnected(true);
        setSocket(ws);

        // Reset retry count on successful connection
        retryCount = 0;
        lastRetryAttempt = 0;

        // Ensure dimensions are correct before sending
        try {
          fitAddon.fit();
          const { cols, rows } = terminal;

          // Get the user's default shell from the environment
          const defaultShell = process.env.SHELL || "/bin/zsh";
          console.log(`Initializing terminal with shell: ${defaultShell}`);

          ws.send(
            JSON.stringify({
              type: "init",
              shell: defaultShell,
              cols,
              rows,
            })
          );
          terminal.writeln("Connecting to terminal server...");
          terminal.focus();
          terminal.writeln("-----");
          terminal.writeln("");
          terminal.writeln("Use the command `tonk guide` if you need help");
          terminal.writeln("");

          // Add a small delay to ensure the shell is ready before sending the command
          setTimeout(() => {
            ws.send(
              JSON.stringify({
                type: "command",
                command: "tonk markdown README.md\r",
              })
            );
          }, 500);
        } catch (e) {
          console.error("Error during WebSocket initialization:", e);
          terminal.writeln(
            "\r\n\x1b[31mError initializing terminal: " + e.message + "\x1b[0m"
          );
        }
      };

      ws.onmessage = (event) => {
        if (!terminal.element) return;

        try {
          let data;
          try {
            data = JSON.parse(event.data);
          } catch (e) {
            data = { type: "output", data: event.data };
          }

          console.log("Received message:", data);

          if (data.type === "output" && data.data) {
            terminal.write(data.data);
          } else if (data.type === "exit") {
            console.error("Terminal exited:", data.data);
            terminal.writeln(data.data);
            setIsConnected(false);
          } else if (data.type === "error") {
            console.error("Terminal error:", data.data);
            terminal.writeln(`\r\n\x1b[31m${data.data}\x1b[0m`);
          } else if (data.output) {
            terminal.write(data.output);
          }
        } catch (error) {
          console.error("Error processing terminal message:", error);
          terminal.writeln(
            `\r\n\x1b[31mError processing message: ${error.message}\x1b[0m`
          );
        }
      };

      ws.onerror = (error) => {
        if (!terminal.element) return;
        console.error("Terminal WebSocket error:", error);
        setIsConnected(false);
        terminal.writeln(
          `\r\n\x1b[31mWebSocket error: ${JSON.stringify(error)}\x1b[0m`
        );
      };

      ws.onclose = () => {
        if (!terminal.element) return;
        console.log("Disconnected from Terminal WebSocket server");
        setIsConnected(false);
        setSocket(null);
        terminal.writeln(
          "\r\n\x1b[31mDisconnected from terminal server\x1b[0m"
        );
        scheduleRetry();
      };

      return ws;
    };

    const ws = connectWebSocket();

    return () => {
      clearTimeout(retryTimeout);
      ws.close();
    };
  }, [terminal, fitAddon, isTerminalReady]);

  // Handle window resize
  useEffect(() => {
    if (
      !fitAddon ||
      !terminal ||
      !socket ||
      !isConnected ||
      !terminal.element ||
      !isTerminalReady
    )
      return;

    let resizeTimeout: NodeJS.Timeout;
    let lastDimensions = { cols: terminal.cols, rows: terminal.rows };

    const handleResize = () => {
      if (!terminal.element) return;

      // Debounce resize events
      clearTimeout(resizeTimeout);
      resizeTimeout = setTimeout(() => {
        try {
          // Get container dimensions
          const containerWidth = terminalContainerRef.current?.clientWidth || 0;
          const containerHeight =
            terminalContainerRef.current?.clientHeight || 0;

          // Only resize if container dimensions are valid
          if (containerWidth < 50 || containerHeight < 50) {
            console.warn("Invalid container dimensions, skipping resize");
            return;
          }

          fitAddon.fit();
          const { cols, rows } = terminal;

          // Only send resize if dimensions actually changed
          if (cols !== lastDimensions.cols || rows !== lastDimensions.rows) {
            lastDimensions = { cols, rows };
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
      }, 100); // Increased debounce timeout
    };

    // Initial fit
    handleResize();

    window.addEventListener("resize", handleResize);

    // Use ResizeObserver with a more conservative approach
    const resizeObserver = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry && entry.contentRect) {
        const { width, height } = entry.contentRect;
        if (width > 0 && height > 0) {
          handleResize();
        }
      }
    });

    if (terminalContainerRef.current) {
      resizeObserver.observe(terminalContainerRef.current);
    }

    return () => {
      clearTimeout(resizeTimeout);
      window.removeEventListener("resize", handleResize);
      resizeObserver.disconnect();
    };
  }, [fitAddon, terminal, socket, isConnected, isTerminalReady]);

  return (
    <div className={styles.container}>
      <div
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
