import { readFileSync } from "fs";
import { resolve } from "path";
import { LLMRequest } from "../ai/src/services/claudeCodeProvider";

export interface TemplateMetadata {
  allowedTools?: string[];
  disallowedTools?: string[];
  systemPrompt?: string;
  permissionMode?: "default" | "acceptEdits" | "bypassPermissions" | "plan";
  maxTurns?: number;
  mcpConfig?: Record<string, any>;
}

export interface ProcessedTemplate {
  content: string;
  metadata: TemplateMetadata;
}

export class TemplateProcessor {
  private templateCache = new Map<string, ProcessedTemplate>();

  /**
   * Process a template file and extract metadata
   */
  async processTemplate(templatePath: string): Promise<ProcessedTemplate> {
    // Check cache first
    if (this.templateCache.has(templatePath)) {
      return this.templateCache.get(templatePath)!;
    }

    const resolvedPath = resolve(templatePath);
    const content = readFileSync(resolvedPath, "utf-8");

    const processed = this.parseTemplate(content);
    this.templateCache.set(templatePath, processed);

    return processed;
  }

  /**
   * Parse template content and extract metadata
   */
  private parseTemplate(content: string): ProcessedTemplate {
    const metadata: TemplateMetadata = {};
    let processedContent = content;

    // Look for YAML frontmatter
    const frontmatterMatch = content.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/);
    if (frontmatterMatch) {
      const [, yamlContent, templateContent] = frontmatterMatch;
      if (templateContent !== undefined) {
        processedContent = templateContent;
      }

      // Simple YAML parsing for our specific needs
      if (yamlContent) {
        this.parseYamlContent(yamlContent, metadata);
      }
    } else {
      // Look for structured comments in markdown
      metadata.allowedTools = this.extractToolsFromContent(content);
      metadata.systemPrompt = this.extractSystemPromptFromContent(content);
    }

    return {
      content: processedContent,
      metadata,
    };
  }

  /**
   * Parse YAML content with support for multi-line objects
   */
  private parseYamlContent(yamlContent: string, metadata: TemplateMetadata) {
    const lines = yamlContent.split('\n');
    let i = 0;
    
    while (i < lines.length) {
      const line = lines[i].trim();
      if (!line || line.startsWith('#')) {
        i++;
        continue;
      }
      
      const colonIndex = line.indexOf(':');
      if (colonIndex === -1) {
        i++;
        continue;
      }
      
      const key = line.substring(0, colonIndex).trim();
      const valueStart = line.substring(colonIndex + 1).trim();
      
      if (valueStart === '{') {
        // Multi-line object - collect until closing brace
        const objLines = ['{'];
        i++;
        let braceCount = 1;
        
        while (i < lines.length && braceCount > 0) {
          const objLine = lines[i].trim();
          objLines.push(objLine);
          
          // Count braces
          for (const char of objLine) {
            if (char === '{') braceCount++;
            if (char === '}') braceCount--;
          }
          
          i++;
        }
        
        const objValue = objLines.join('\n');
        this.parseMetadataField(key, objValue, metadata);
      } else {
        // Single line value
        this.parseMetadataField(key, valueStart, metadata);
        i++;
      }
    }
  }

  /**
   * Parse individual metadata fields
   */
  private parseMetadataField(
    key: string,
    value: string,
    metadata: TemplateMetadata,
  ) {
    switch (key.toLowerCase()) {
      case "allowedtools":
        metadata.allowedTools = this.parseArrayValue(value);
        break;
      case "disallowedtools":
        metadata.disallowedTools = this.parseArrayValue(value);
        break;
      case "systemprompt":
        metadata.systemPrompt = this.parseStringValue(value);
        break;
      case "permissionmode":
        metadata.permissionMode = this.parseStringValue(value) as any;
        break;
      case "maxturns":
        metadata.maxTurns = parseInt(this.parseStringValue(value));
        break;
      case "mcpconfig":
        try {
          // Handle JSON object (multi-line from YAML)
          if (value.trim().startsWith("{")) {
            metadata.mcpConfig = JSON.parse(value);
          } else {
            // Handle simple string value
            const cleanValue = this.parseStringValue(value);
            if (cleanValue.startsWith("{")) {
              metadata.mcpConfig = JSON.parse(cleanValue);
            } else {
              metadata.mcpConfig = {};
            }
          }
        } catch (e) {
          console.warn(`Failed to parse mcpConfig: ${e}`);
          console.warn(`Value was: ${value}`);
          metadata.mcpConfig = {};
        }
        break;
    }
  }

  /**
   * Parse array values from YAML-like strings
   */
  private parseArrayValue(value: string): string[] {
    if (value.startsWith("[") && value.endsWith("]")) {
      return value
        .slice(1, -1)
        .split(",")
        .map((s) => s.trim().replace(/["']/g, ""));
    }
    return value.split(",").map((s) => s.trim());
  }

  /**
   * Parse string values and remove quotes
   */
  private parseStringValue(value: string): string {
    return value.replace(/^["']|["']$/g, "");
  }

  /**
   * Extract tools from template content by analyzing mentions
   */
  private extractToolsFromContent(content: string): string[] {
    const tools: string[] = [];

    // Look for tool mentions in the content
    if (
      content.includes("browsermcp") ||
      content.includes("browser") ||
      content.includes("web")
    ) {
      tools.push("browsermcp");
    }

    if (content.includes("gmail") || content.includes("email")) {
      tools.push("gmail");
    }

    // Add more tool detection logic as needed

    return tools;
  }

  /**
   * Extract system prompt from template content
   */
  private extractSystemPromptFromContent(content: string): string {
    // Use the template content itself as the system prompt
    // Remove the "Job To Be Done" section as that will be filled by the user prompt
    const sections = content.split("## Job To Be Done");
    return sections[0]?.trim() || content.trim();
  }

  /**
   * Create an LLMRequest from template and user prompt
   */
  async createLLMRequest(
    templatePath: string,
    userPrompt: string,
    context?: string,
  ): Promise<LLMRequest> {
    const template = await this.processTemplate(templatePath);

    let finalPrompt = userPrompt;
    if (context) {
      finalPrompt = `Context from previous jobs:\n${context}\n\nTask: ${userPrompt}`;
    }

    // Use the template content as the system prompt
    const systemPrompt = template.content;

    const request: LLMRequest = {
      prompt: finalPrompt,
      systemPrompt: systemPrompt,
      ...(template.metadata.allowedTools && {
        allowedTools: template.metadata.allowedTools,
      }),
      ...(template.metadata.disallowedTools && {
        disallowedTools: template.metadata.disallowedTools,
      }),
      ...(template.metadata.permissionMode && {
        permissionMode: template.metadata.permissionMode,
      }),
      ...(template.metadata.maxTurns && {
        maxTurns: template.metadata.maxTurns,
      }),
      ...(template.metadata.mcpConfig && {
        mcpConfig: template.metadata.mcpConfig,
      }),
    };

    return request;
  }
}

