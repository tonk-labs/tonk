// This file handles the renderer process logic

// DOM Elements
const createBtn = document.getElementById('createBtn');
const devBtn = document.getElementById('devBtn');
const serveBtn = document.getElementById('serveBtn');
const deployBtn = document.getElementById('deployBtn');
const configBtn = document.getElementById('configBtn');
const outputArea = document.getElementById('outputArea');
const projectsList = document.getElementById('projectsList');

// Helper function to append output
function appendOutput(text) {
  const timestamp = new Date().toLocaleTimeString();
  outputArea.innerHTML += `\n[${timestamp}] ${text}`;
  outputArea.scrollTop = outputArea.scrollHeight;
}

// Command execution
async function executeCommand(command) {
  appendOutput(`Executing: tonk ${command}`);
  try {
    const result = await window.tonkAPI.executeCommand(command);
    appendOutput(result);
  } catch (error) {
    appendOutput(`Error: ${error.message}`);
  }
}

// Button event listeners
createBtn.addEventListener('click', () => {
  appendOutput('Starting project creation wizard...');
  // Here you would implement a form or dialog for project creation
  // For now, we'll just simulate it
  setTimeout(() => {
    appendOutput('Project creation wizard would appear here');
  }, 500);
});

devBtn.addEventListener('click', () => {
  executeCommand('dev');
});

serveBtn.addEventListener('click', () => {
  executeCommand('serve');
});

deployBtn.addEventListener('click', () => {
  executeCommand('deploy');
});

configBtn.addEventListener('click', () => {
  executeCommand('config');
});

// Initialize the app
function init() {
  appendOutput('Tonk GUI initialized');

  // Here you would scan for existing projects and populate the list
  // For now, we'll just add a placeholder
  appendOutput('Scanning for existing Tonk projects...');

  // Simulate loading projects
  setTimeout(() => {
    appendOutput('Project scanning complete');
  }, 1000);
}

// Run initialization
init();
