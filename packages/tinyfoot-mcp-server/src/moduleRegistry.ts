import {ParsedFunction} from './utils/parser.js';
import {getEmbedding, cosineSimilarity} from './utils/vector.js';
import {FSWatcher} from 'chokidar';
import path from 'path';
import {parseModuleFile} from './utils/parser.js';
import fs from 'fs/promises';
import express, {Request, Response, RequestHandler} from 'express';
import chokidar from 'chokidar';

interface ModuleWithEmbedding extends ParsedFunction {
  embedding: number[];
  filePath: string;
}

export class ModuleRegistry {
  private modules: Map<string, ModuleWithEmbedding> = new Map();
  private watcher: FSWatcher | null = null;
  private app = express();
  private server: ReturnType<typeof this.app.listen> | null = null;
  private port = 4321;

  constructor(private modulesDir: string) {
    console.log(
      `[ModuleRegistry] Initializing with modules directory: ${modulesDir}`,
    );
    this.setupServer();
  }

  private setupServer() {
    this.app.use(express.json());

    // Endpoint to find similar modules
    this.app.post('/find-similar', (async (req: Request, res: Response) => {
      try {
        const {query, threshold = 0.3} = req.body;
        if (!query) {
          return res.status(400).json({error: 'Query is required'});
        }
        const results = await this.findSimilarModules(query, threshold);
        res.json({results});
      } catch (error) {
        console.error('Error finding similar modules:', error);
        res.status(500).json({error: 'Internal server error'});
      }
    }) as RequestHandler);

    // Start the server
    this.server = this.app.listen(this.port, () => {
      console.log(`[ModuleRegistry] Server listening on port ${this.port}`);
    });
  }

  async initialize() {
    console.log('[ModuleRegistry] Starting initialization...');
    // Initial scan of modules directory
    await this.scanModules();

    // Set up file watcher
    console.log('[ModuleRegistry] Setting up file watcher...');
    this.watcher = chokidar.watch(path.join(this.modulesDir, '**/*.{ts,tsx}'), {
      ignored: /(^|[/\\])\./, // ignore dotfiles
      persistent: true,
    });

    await new Promise<void>(resolve => {
      this.watcher!.on('add', (filePath: string) =>
        this.handleFileChange(filePath),
      )
        .on('change', (filePath: string) => this.handleFileChange(filePath))
        .on('unlink', (filePath: string) => this.handleFileDelete(filePath))
        .on('ready', () => resolve());
    });

    console.log(
      `[ModuleRegistry] Initialization complete. Found ${this.modules.size} modules.`,
    );
  }

  private async scanModules() {
    console.log('[ModuleRegistry] Starting module scan...');
    try {
      const files = await this.getAllModuleFiles(this.modulesDir);
      console.log(
        `[ModuleRegistry] Found ${files.length} module files to process`,
      );
      for (const file of files) {
        await this.processModuleFile(file);
      }
    } catch (error) {
      console.error('[ModuleRegistry] Error scanning modules:', error);
    }
  }

  private async getAllModuleFiles(dir: string): Promise<string[]> {
    const files: string[] = [];

    try {
      const entries = await fs.readdir(dir, {withFileTypes: true});
      console.log(`[ModuleRegistry] Scanning directory: ${dir}`);

      for (const entry of entries) {
        const fullPath = path.join(dir, entry.name);

        if (entry.isDirectory()) {
          files.push(...(await this.getAllModuleFiles(fullPath)));
        } else if (
          entry.isFile() &&
          (entry.name.endsWith('.ts') || entry.name.endsWith('.tsx'))
        ) {
          files.push(fullPath);
        }
      }
    } catch (error) {
      console.error(`[ModuleRegistry] Error reading directory ${dir}:`, error);
    }

    return files;
  }

  private async processModuleFile(filePath: string) {
    console.log(`[ModuleRegistry] Processing module file: ${filePath}`);
    try {
      const parsedModules = await parseModuleFile(filePath);
      if (!parsedModules) {
        console.log(`[ModuleRegistry] No modules found in file: ${filePath}`);
        return;
      }

      console.log(
        `[ModuleRegistry] Found ${parsedModules.length} modules in ${filePath}`,
      );
      // Get embeddings for each module
      for (const module of parsedModules) {
        const moduleText = `${module.name} ${module.description}`;
        console.log(
          `[ModuleRegistry] Generating embedding for module: ${module.name}`,
        );
        const embedding = await getEmbedding(moduleText);

        // Store module with its embedding
        this.modules.set(module.name, {
          ...module,
          embedding,
          filePath,
        });
        console.log(
          `[ModuleRegistry] Successfully processed module: ${module.name}`,
        );
      }
    } catch (error) {
      console.error(
        `[ModuleRegistry] Error processing module file ${filePath}:`,
        error,
      );
    }
  }

  private async handleFileChange(filePath: string) {
    console.log(`[ModuleRegistry] File changed: ${filePath}`);
    await this.processModuleFile(filePath);
  }

  private handleFileDelete(filePath: string) {
    console.log(`[ModuleRegistry] File deleted: ${filePath}`);
    // Remove any modules that were defined in this file
    let deletedCount = 0;
    for (const [name, module] of this.modules.entries()) {
      if (module.filePath === filePath) {
        this.modules.delete(name);
        deletedCount++;
      }
    }
    console.log(
      `[ModuleRegistry] Removed ${deletedCount} modules from deleted file`,
    );
  }

  async findSimilarModules(
    query: string,
    threshold = 0.3,
  ): Promise<ParsedFunction[]> {
    console.log(
      `[ModuleRegistry] Searching for modules similar to: "${query}"`,
    );
    try {
      const queryEmbedding = await getEmbedding(query);
      console.log('[ModuleRegistry] Generated query embedding');

      const results = Array.from(this.modules.values())
        .map(module => ({
          module,
          similarity: cosineSimilarity(queryEmbedding, module.embedding),
        }))
        .filter(({similarity}) => similarity >= threshold)
        .sort((a, b) => b.similarity - a.similarity)
        .map(({module}) => ({
          name: module.name,
          description: module.description,
        }));

      console.log(
        `[ModuleRegistry] Found ${results.length} similar modules (threshold: ${threshold})`,
      );
      return results;
    } catch (error) {
      console.error('[ModuleRegistry] Error finding similar modules:', error);
      return [];
    }
  }

  async dispose() {
    console.log('[ModuleRegistry] Starting cleanup...');

    if (this.watcher) {
      console.log('[ModuleRegistry] Closing file watcher...');
      await this.watcher.close();
      this.watcher = null;
    }

    if (this.server) {
      console.log('[ModuleRegistry] Closing server...');
      this.server.close();
      this.server = null;
    }

    this.modules.clear();
    console.log('[ModuleRegistry] Cleanup complete');
  }
}
