const WebSocket = require('ws');
const os = require('os');
const pty = require('node-pty');
const fs = require('fs');
const path = require('path');

const run = (port, dirPath) => {
  // Create WebSocket server
  const wss = new WebSocket.Server({ port });
  console.log(dirPath);

  console.log(`Terminal WebSocket server running on ws://localhost:${port}`);

  wss.on('connection', (ws) => {
    console.log('Client connected');
    
    let ptyProcess = null;

    ws.on('message', (message) => {
      try {
        const data = JSON.parse(message);
        
        switch (data.type) {
          case 'init':
            // Initialize new terminal session
            const shell = process.platform === 'win32' 
              ? 'powershell.exe' 
              : data.shell || process.env.SHELL || 'bash';
            
            // const shellArgs = process.platform === 'win32' ? [] : [];
            const shellArgs = [];
            
            // Verify shell exists
            let shellPath = shell;
            if (!path.isAbsolute(shell)) {
              // Common paths where shells might be found
              const commonPaths = ['/bin', '/usr/bin', '/usr/local/bin'];
              for (const basePath of commonPaths) {
                const fullPath = path.join(basePath, shell);
                if (fs.existsSync(fullPath)) {
                  shellPath = fullPath;
                  break;
                }
              }
            }

            // Verify working directory exists
            const workingDir = dirPath || process.env.HOME;
            if (!fs.existsSync(workingDir)) {
              console.error(`Working directory does not exist: ${workingDir}`);
              ws.send(JSON.stringify({
                type: 'error',
                data: `Working directory does not exist: ${workingDir}`
              }));
              return;
            }

            console.log(`Spawning terminal with:
  - Shell path: ${shellPath}
  - Shell args: ${shellArgs.join(' ')}
  - Working dir: ${workingDir}
  - Cols: ${data.cols || 80}
  - Rows: ${data.rows || 24}
`);
            
            try {
              ptyProcess = pty.spawn(shellPath, shellArgs, {
                name: 'xterm-color',
                cols: data.cols || 80,
                rows: data.rows || 24,
                cwd: workingDir,
                env: {
                  ...process.env,
                  TERM: 'xterm-256color',
                  PWD: workingDir,
                  SHELL: shellPath
                }
              });

              // Handle terminal output
              ptyProcess.onData((data) => {
                try {
                  ws.send(JSON.stringify({ type: 'output', data }));
                } catch (err) {
                  console.error('Error sending terminal output:', err);
                }
              });

              // Handle terminal exit
              ptyProcess.onExit(({ exitCode, signal }) => {
                console.error(`Shell process exited with code ${exitCode} (signal: ${signal})`);
                try {
                  ws.send(JSON.stringify({
                    type: 'exit',
                    data: `\r\nProcess exited with code ${exitCode} (signal: ${signal})`
                  }));
                } catch (err) {
                  console.error('Error sending exit message:', err);
                }
              });

              // Send initial confirmation
              ws.send(JSON.stringify({ 
                type: 'output', 
                data: `\r\nTerminal initialized with ${shellPath}\r\n` 
              }));

            } catch (error) {
              console.error('Failed to spawn terminal process:', error);
              ws.send(JSON.stringify({
                type: 'error',
                data: `Failed to spawn terminal: ${error.message}`
              }));
            }
            break;

          case 'command':
            // Send command to terminal
            if (ptyProcess) {
              ptyProcess.write(data.command);
            }
            break;
          case 'resize':
            // Resize terminal
            if (ptyProcess) {
              ptyProcess.resize(data.cols, data.rows);
            }
            break;
        }
      } catch (error) {
        console.error('Error processing message:', error);
        ws.send(JSON.stringify({
          type: 'error',
          data: `Error: ${error.message}`
        }));
      }
    });

    // Handle client disconnect
    ws.on('close', () => {
      console.log('Client disconnected');
      if (ptyProcess) {
        ptyProcess.kill();
        ptyProcess = null;
      }
    });

    // Handle errors
    ws.on('error', (error) => {
      console.error('WebSocket error:', error);
    });
  });

  return wss;
}

module.exports = {
  run,
}