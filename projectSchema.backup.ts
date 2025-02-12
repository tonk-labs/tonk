/**
 * @title Project Schema
 * @description Global context definition for AI-assisted project management.
 * This schema acts as a central knowledge base for LLMs to understand and 
 * manipulate project structure, components, and relationships.
 */
export interface ProjectSchema {
  /**
   * Core project metadata and configuration
   */
  metadata: {
    /** Project name as defined in package.json */
    name: string;
    /** Project version following semver */
    version: string;
    /** Technologies and frameworks in use */
    stack: {
      framework: "next" | "react" | "vue" | "svelte";
      styling: "tailwind" | "styled-components" | "css-modules";
      database?: "postgres" | "mysql" | "mongodb";
    };
  };

  /**
   * Component definitions with their relationships and responsibilities
   */
  components: {
    /** Unique identifier for the component */
    [id: string]: {
      /** Component display name */
      name: string;
      /** File path relative to project root */
      path: string;
      /** Component's primary responsibility */
      purpose: string;
      /** Parent component if this is a child */
      parent?: string;
      /** Child components contained within this one */
      children?: string[];
      /** Props interface or type definition */
      props?: string;
      /** State management details */
      state?: {
        /** Local state variables */
        local?: {
          name: string;
          type: string;
          purpose: string;
        }[];
        /** Global state dependencies */
        global?: {
          store: string;
          selectors: string[];
        }[];
      };
    };
  };

  /**
   * Global state stores and their schemas
   */
  state: {
    [storeName: string]: {
      /** Store's primary purpose */
      purpose: string;
      /** State shape definition */
      schema: Record<string, unknown>;
      /** Actions that can modify this store */
      actions: {
        name: string;
        description: string;
        parameters?: Record<string, string>;
      }[];
    };
  };

  /**
   * API endpoints and their contracts
   */
  api: {
    [endpoint: string]: {
      /** HTTP method */
      method: "GET" | "POST" | "PUT" | "DELETE" | "PATCH";
      /** Path with parameter template */
      path: string;
      /** Expected request body type */
      requestSchema?: Record<string, unknown>;
      /** Expected response type */
      responseSchema: Record<string, unknown>;
      /** Authentication requirements */
      auth?: boolean;
    };
  };

  /**
   * Database schema definitions
   */
  database?: {
    [model: string]: {
      /** Model fields and their types */
      fields: Record<string, string>;
      /** Relationships to other models */
      relations: {
        [fieldName: string]: {
          type: "hasOne" | "hasMany" | "belongsTo" | "manyToMany";
          model: string;
          foreignKey: string;
        };
      };
    };
  };

  /**
   * Third-party integrations and their configurations
   */
  integrations: {
    [serviceName: string]: {
      /** Service purpose */
      purpose: string;
      /** Configuration requirements */
      config: Record<string, string>;
      /** Available methods/actions */
      methods: {
        name: string;
        description: string;
        parameters?: Record<string, string>;
        returnType?: string;
      }[];
    };
  };

  /**
   * Custom functions and utilities
   */
  utils: {
    [functionName: string]: {
      /** Function purpose */
      description: string;
      /** Location in project */
      path: string;
      /** Expected parameters */
      parameters?: Record<string, string>;
      /** Return type */
      returnType?: string;
      /** Usage examples */
      examples?: string[];
    };
  };

  /**
   * AI context and preferences for code generation
   */
  aiContext: {
    /** Preferred code style */
    codeStyle: {
      /** Naming conventions */
      naming: {
        components: "PascalCase";
        functions: "camelCase";
        constants: "UPPERCASE";
      };
      /** Documentation preferences */
      documentation: {
        requireJSDoc: boolean;
        requireExamples: boolean;
        requireParamDescriptions: boolean;
      };
    };
    /** Project-specific generation rules */
    generationRules: {
      /** Required patterns or anti-patterns */
      patterns: {
        /** Pattern name */
        name: string;
        /** Pattern description */
        description: string;
        /** Example implementation */
        example?: string;
      }[];
    };
  };
}
