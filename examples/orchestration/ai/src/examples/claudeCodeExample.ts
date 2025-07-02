import {
  ClaudeCodeProvider,
  MCPServerConfig,
  LLMRequest,
} from "../services/claudeCodeProvider";

/**
 * Simple Claude Code example with direct prompts
 */
async function claudeCodeExample() {
  // Define MCP server configuration for external tools
  const mcpConfig: MCPServerConfig = {
    filesystem: {
      command: "npx",
      args: [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "/path/to/allowed/files",
      ],
    },
    github: {
      command: "npx",
      args: ["-y", "@modelcontextprotocol/server-github"],
      env: {
        GITHUB_TOKEN: process.env.GITHUB_TOKEN || "",
      },
    },
  };

  // Create Claude Code provider with simple configuration
  const claudeCodeProvider = new ClaudeCodeProvider({
    systemPrompt:
      "You are a helpful coding assistant with access to local tools.",
    maxTurns: 5,
    permissionMode: "default",
    allowedTools: [
      "read_file",
      "write_file",
      "edit_file",
      "list_files",
      "bash",
    ],
    disallowedTools: ["rm", "sudo", "curl"],
    mcpConfig,
    workingDirectory: process.cwd(),
    verbose: true,
  });

  console.log("🚀 Starting Claude Code example...\n");

  try {
    // Example 1: Simple code analysis
    console.log("📋 Example 1: Code Analysis");
    const analysisResponse = await claudeCodeProvider.complete({
      prompt:
        "Analyze the files in the current directory and suggest improvements to the project structure.",
    });

    console.log("📊 Analysis Result:");
    console.log(analysisResponse.content);
    if (analysisResponse.totalCostUsd) {
      console.log(`💰 Cost: $${analysisResponse.totalCostUsd.toFixed(4)}\n`);
    }

    // Example 2: Code generation with file operations
    console.log("📋 Example 2: Code Generation");
    const generationResponse = await claudeCodeProvider.complete({
      prompt:
        "Create a new TypeScript utility function for date formatting and save it to a new file. Include proper types and documentation.",
    });

    console.log("🔧 Generation Result:");
    console.log(generationResponse.content);

    // Example 3: Streaming response for long-running tasks
    console.log("\n📋 Example 3: Streaming Response");
    console.log("🔄 Streaming response for project setup...");

    let streamContent = "";
    for await (const chunk of claudeCodeProvider.stream({
      prompt:
        "Set up a new Node.js project with TypeScript, create a basic package.json, and initialize git. Explain each step as you do it.",
    })) {
      process.stdout.write(chunk);
      streamContent += chunk;
    }

    console.log("\n\n✅ Examples completed successfully!");
  } catch (error) {
    console.error("❌ Error during Claude Code example:", error);

    if (error instanceof Error) {
      if (error.message.includes("Claude Code SDK not found")) {
        console.log("\n💡 To fix this error:");
        console.log(
          "1. Install Claude Code CLI: npm install -g @anthropic-ai/claude-code"
        );
        console.log(
          "2. Install the SDK: npm install @anthropic-ai/claude-code"
        );
        console.log(
          "3. Authenticate: Run 'claude' and follow the OAuth process"
        );
      } else if (error.message.includes("Tool usage denied")) {
        console.log("\n💡 Permission denied:");
        console.log("- This is expected for dangerous operations");
        console.log("- Claude Code's permission system is working correctly");
        console.log(
          "- You can modify allowedTools/disallowedTools to change behavior"
        );
      }
    }
  }
}

/**
 * Permission configuration examples
 */
async function permissionConfigurationExample() {
  console.log("\n🔧 Permission Configuration Examples\n");

  // Restrictive permissions
  const restrictiveProvider = new ClaudeCodeProvider({
    systemPrompt: "You are a code analysis assistant. You can only read files.",
    allowedTools: ["read_file", "list_files"],
    disallowedTools: ["write_file", "edit_file", "bash"],
    permissionMode: "bypassPermissions",
  });

  console.log("🔒 Restrictive provider: Only file reading allowed");

  // Development environment
  const devProvider = new ClaudeCodeProvider({
    systemPrompt:
      "You are a development assistant with file editing permissions.",
    allowedTools: ["read_file", "write_file", "edit_file", "bash"],
    disallowedTools: ["rm", "sudo", "curl"],
    permissionMode: "acceptEdits",
  });

  console.log(
    "🛠️ Development provider: Auto-accepts edits, prompts for commands"
  );

  // Fully controlled environment
  const controlledProvider = new ClaudeCodeProvider({
    systemPrompt: "You are a trusted assistant in a sandboxed environment.",
    permissionMode: "bypassPermissions",
    maxTurns: 10,
  });

  console.log("🌐 Controlled provider: All operations allowed without prompts");

  console.log("\n📝 Permission Mode Options:");
  console.log("- 'default': Prompt for all potentially dangerous operations");
  console.log("- 'acceptEdits': Auto-approve file edits, prompt for commands");
  console.log("- 'bypassPermissions': Skip all permission prompts");
  console.log("- 'plan': Show execution plan before running");
}

/**
 * Simple streaming example
 */
async function streamingExample() {
  console.log("\n🌊 Streaming Example\n");

  const provider = new ClaudeCodeProvider({
    systemPrompt: "You are a helpful assistant.",
    permissionMode: "default",
    outputFormat: "stream-json",
  });

  console.log("💬 Streaming conversation:");

  for await (const chunk of provider.stream({
    prompt:
      "Write a simple Python script that prints 'Hello, World!' and explain what it does.",
  })) {
    process.stdout.write(chunk);
  }

  console.log("\n\n✅ Streaming completed!");
}

// Export examples for use
export { claudeCodeExample, permissionConfigurationExample, streamingExample };

// Run the main example if called directly
if (require.main === module) {
  claudeCodeExample()
    .then(() => permissionConfigurationExample())
    .then(() => streamingExample())
    .catch(console.error);
}
