import { Terminal as XTerm } from "xterm";
import { FitAddon } from "xterm-addon-fit";
import { WebLinksAddon } from "xterm-addon-web-links";
import { FileType } from "../Tree";
import { getConfig } from "../../ipc/config";
import { platformSensitiveJoin } from "../../ipc/files";

export interface TerminalOptions {
  container: HTMLDivElement;
}

type Directory = {
  fileType: FileType;
  name: string;
};

export class TerminalManager {
  private terminal: XTerm | null = null;
  private fitAddon: FitAddon | null = null;
  private socket: WebSocket | null = null;
  private isConnected: boolean = false;
  private isTerminalReady: boolean = false;
  private containerRef: HTMLDivElement | null = null;
  private onConnectionChange: (isConnected: boolean) => void = () => {};
  private cmdNonce = 0;
  private callbacks: {
    [key: string]: () => void;
  } = {};

  constructor(options: TerminalOptions) {
    this.containerRef = options.container;
  }

  setOnConnectionChange(callback: (isConnected: boolean) => void): void {
    this.onConnectionChange = callback;
  }

  async initialize(): Promise<void> {
    if (!this.containerRef) return;

    // Clear any existing terminal
    this.containerRef.innerHTML = "";

    // Create new terminal instance
    this.terminal = new XTerm({
      cursorBlink: true,
      fontFamily: '"Courier New", monospace',
      fontSize: 14,
      rows: 24,
      cols: 80,
      theme: {
        background: "rgba(30, 30, 30, 0.8)", // Semi-transparent background
        foreground: "#f0f0f0",
        cursor: "#f0f0f0",
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
      allowTransparency: true, // Enable transparency support
    });

    // Create and register fit addon
    this.fitAddon = new FitAddon();
    this.terminal.loadAddon(this.fitAddon);

    // Add web links addon
    this.terminal.loadAddon(new WebLinksAddon());

    // Open the terminal in the container
    this.terminal.open(this.containerRef);

    // Wait for the terminal to be ready
    await this.waitForTerminalReady();

    // Initial fit
    try {
      this.fitAddon.fit();
      this.terminal.focus();
      this.isTerminalReady = true;
    } catch (e) {
      console.error("Error during initial terminal fit:", e);
    }

    // Setup data listener
    this.setupDataListener();

    // Setup resize handler
    this.setupResizeHandler();
    this.terminal.writeln("Connecting to terminal server...");
    this.terminal.focus();
    this.terminal.writeln("-----");
    this.terminal.writeln("");
    this.terminal.writeln("Use the command `tonk guide` if you need help");
    this.terminal.writeln("");
  }

  private async waitForTerminalReady(): Promise<boolean> {
    if (!this.terminal || !this.containerRef) return false;

    return new Promise((resolve) => {
      const checkReady = () => {
        if (
          this.terminal?.element &&
          (this.containerRef?.offsetHeight ?? 0) > 0
        ) {
          resolve(true);
        } else {
          setTimeout(checkReady, 100);
        }
      };
      checkReady();
    });
  }

  private setupDataListener(): void {
    if (!this.terminal) return;

    this.terminal.onData((data: string) => {
      if (this.socket && this.isConnected) {
        this.socket.send(
          JSON.stringify({
            type: "command",
            command: data,
          }),
        );
      }
    });
  }

  async connectToItem(selectedItem: Directory): Promise<void> {
    if (!this.terminal || !this.isTerminalReady) return;

    // Close any existing connection first
    await this.closeConnection();

    await this.connectWebSocket(selectedItem);
  }

  private async closeConnection(): Promise<void> {
    if (this.socket) {
      // Send CTRL+C to terminate any running processes
      if (this.isConnected) {
        this.socket.send(
          JSON.stringify({
            type: "command",
            command: "\x03",
          }),
        );
      }
      // Small delay to ensure termination signal is processed
      await new Promise((resolve) => setTimeout(resolve, 100));
      if (this.socket) {
        this.socket.close();
        this.socket = null;
        this.setConnected(false);
      }
    } else {
      this.setConnected(false);
    }
  }

  private async connectWebSocket(selectedItem: Directory): Promise<void> {
    // Get the WebSocket URL based on the environment
    console.log("connecting to", selectedItem);
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const hostname = window.location.hostname || "localhost";
    const wsUrl = `${protocol}//${hostname}:3060`;
    const ws = new WebSocket(wsUrl);

    ws.onopen = () => this.handleWebSocketOpen(ws, selectedItem);
    ws.onmessage = (event) => this.handleWebSocketMessage(selectedItem, event);
    ws.onerror = (error) => this.handleWebSocketError(error);
    ws.onclose = () => this.handleWebSocketClose();

    this.terminal?.clear();

    this.socket = ws;
    this.onConnectionChange(true);
  }

  async handleWebSocketOpen(
    ws: WebSocket,
    selectedItem: Directory,
  ): Promise<void> {
    if (!this.terminal?.element || !this.fitAddon) return;
    console.log("handleWebSocketOpen", selectedItem);

    this.setConnected(true);

    // Ensure dimensions are correct before sending
    try {
      this.fitAddon.fit();
      const { cols, rows } = this.terminal;

      // Get the user's default shell from the environment
      const defaultShell = process.env.SHELL || "/bin/zsh";

      ws.send(
        JSON.stringify({
          type: "init",
          shell: defaultShell,
          cols,
          rows,
        }),
      );

      let isInit = false;

      if (selectedItem.fileType === FileType.App) {
        const config = await getConfig();
        const subPath = selectedItem.name.split("/");
        const fullPath = await platformSensitiveJoin([
          config!.homePath,
          "apps",
          ...subPath,
        ]);
        if (fullPath) {
          try {
            const files = await window.electronAPI.ls(fullPath);
            const found = files.length === 0;
            if (found) {
              isInit = true;
            }
          } catch (e: any) {
            console.error("Error during WebSocket initialization:", e);
            this.terminal.writeln(
              "\r\n\x1b[31mError initializing terminal: " +
                e.message +
                "\x1b[0m",
            );
          }
        }
      }

      await new Promise((resolve) => setTimeout(resolve, 1000));
      if (isInit) {
        ws.send(
          JSON.stringify({
            type: "command",
            command: "tonk create --init\r",
          }),
        );
      } else {
        // Add a small delay to ensure the shell is ready before sending the command
        ws.send(
          JSON.stringify({
            type: "command",
            command: "pnpm install && tonk markdown README.md\r",
          }),
        );
      }
    } catch (e: unknown) {
      if (e instanceof Error) {
        console.error("Error during WebSocket initialization:", e);
        this.terminal.writeln(
          "\r\n\x1b[31mError initializing terminal: " + e.message + "\x1b[0m",
        );
      }
    }
  }

  private async handleWebSocketMessage(
    location: Directory,
    event: MessageEvent,
  ): Promise<void> {
    if (!this.terminal?.element) return;
    console.log("handleWebSocketMessage", location, event);

    try {
      let data;
      try {
        data = JSON.parse(event.data);
      } catch (e) {
        data = { type: "output", data: event.data };
      }

      if (data.type === "output" && data.data) {
        this.terminal.write(data.data);
      } else if (data.type === "exit") {
        console.error("Terminal exited:", data.data);
        this.terminal.writeln(data.data);
        this.setConnected(false);
      } else if (data.type === "error") {
        console.error("Terminal error:", data.data);
        this.terminal.writeln(`\r\n\x1b[31m${data.data}\x1b[0m`);
        // await this.closeConnection();
        // await this.connectWebSocket(location);
      } else if (data.type === "cmdFinished") {
        if (this.callbacks[data.id]) {
          this.callbacks[data.id]();
          delete this.callbacks[data.id];
        }
      } else if (data.output) {
        this.terminal.write(data.output);
      }
    } catch (error: unknown) {
      if (error instanceof Error) {
        console.error("Error processing terminal message:", error);
        this.terminal.writeln(
          `\r\n\x1b[31mError processing message: ${error.message}\x1b[0m`,
        );
      }
    }
  }

  private handleWebSocketError(error: Event): void {
    if (!this.terminal?.element) return;
    console.error("Terminal WebSocket error:", error);
    this.setConnected(false);
    this.terminal.writeln(
      `\r\n\x1b[31mWebSocket error: ${JSON.stringify(error)}\x1b[0m`,
    );
  }

  private handleWebSocketClose(): void {
    if (!this.terminal?.element) return;
    this.setConnected(false);
  }

  private setConnected(connected: boolean): void {
    this.isConnected = connected;
    this.onConnectionChange(connected);
  }

  executeCommand(cmd: string, onComplete: () => void): void {
    if (cmd !== "" && this.socket) {
      switch (cmd) {
        case "stopAndReset": {
          this.socket.send(
            JSON.stringify({
              type: "command",
              command: "\x03",
            }),
          );
          this.socket.send(
            JSON.stringify({
              type: "command",
              command: "clear\r",
            }),
          );
          break;
        }
        case "build": {
          const cmdId = this.cmdNonce++;
          this.callbacks[`${cmdId}`] = onComplete;
          this.socket.send(
            JSON.stringify({
              type: "command",
              id: `${cmdId}`,
              command: "pnpm run build",
            }),
          );
        }
        default: {
          console.error("didn't recognize the sent command");
        }
      }
    }
  }

  private setupResizeHandler(): void {
    if (!this.fitAddon || !this.terminal || !this.containerRef) return;

    let resizeTimeout: NodeJS.Timeout;
    let lastDimensions = { cols: this.terminal.cols, rows: this.terminal.rows };

    const handleResize = () => {
      if (!this.terminal?.element) return;

      // Debounce resize events
      clearTimeout(resizeTimeout);
      resizeTimeout = setTimeout(() => {
        try {
          // Get container dimensions
          const containerWidth = this.containerRef?.clientWidth || 0;
          const containerHeight = this.containerRef?.clientHeight || 0;

          // Only resize if container dimensions are valid
          if (containerWidth < 50 || containerHeight < 50) {
            return;
          }

          this.fitAddon?.fit();
          if (!this.terminal) return;

          const { cols, rows } = this.terminal;

          // Only send resize if dimensions actually changed
          if (cols !== lastDimensions.cols || rows !== lastDimensions.rows) {
            lastDimensions = { cols, rows };
            this.socket?.send(
              JSON.stringify({
                type: "resize",
                cols,
                rows,
              }),
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

    if (this.containerRef) {
      resizeObserver.observe(this.containerRef);
    }
  }

  async dispose(): Promise<void> {
    await this.closeConnection();
    if (this.terminal) {
      this.terminal.dispose();
      this.terminal = null;
    }
  }

  get isTerminalConnected(): boolean {
    return this.isConnected;
  }
}
