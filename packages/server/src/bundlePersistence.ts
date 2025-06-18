import fs from 'fs';
import path from 'path';
import chalk from 'chalk';

export interface PersistedBundleRoute {
  bundleName: string;
  bundlePath: string;
  route: string;
  id: string;
  startTime: string;
  isRunning: boolean;
}

export interface BundlePersistenceConfig {
  persistencePath: string;
  verbose?: boolean;
}

export class BundlePersistence {
  private persistenceFile: string;
  private verbose: boolean;

  constructor(config: BundlePersistenceConfig) {
    this.persistenceFile = path.join(config.persistencePath, 'bundles.json');
    this.verbose = config.verbose ?? true;

    // Ensure the persistence directory exists
    const dir = path.dirname(this.persistenceFile);
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, {recursive: true});
    }
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    if (this.verbose) {
      console.log(chalk[color](`[BundlePersistence] ${message}`));
    }
  }

  /**
   * Save running bundle routes to disk
   */
  async saveBundleRoutes(
    bundleRoutes: Map<
      string,
      {
        bundleName: string;
        bundlePath: string;
        route: string;
        id: string;
        startTime: Date;
        isRunning: boolean;
      }
    >,
  ): Promise<void> {
    try {
      // Convert Map to array and serialize dates
      const serializedRoutes: PersistedBundleRoute[] = Array.from(
        bundleRoutes.entries(),
      ).map(([, bundle]) => ({
        bundleName: bundle.bundleName,
        bundlePath: bundle.bundlePath,
        route: bundle.route,
        id: bundle.id,
        startTime: bundle.startTime.toISOString(),
        isRunning: bundle.isRunning,
      }));

      await fs.promises.writeFile(
        this.persistenceFile,
        JSON.stringify(serializedRoutes, null, 2),
        'utf8',
      );

      this.log(
        'blue',
        `Saved ${serializedRoutes.length} bundle routes to persistence`,
      );
    } catch (error) {
      this.log(
        'red',
        `Failed to save bundle routes: ${error instanceof Error ? error.message : 'Unknown error'}`,
      );
      throw error;
    }
  }

  /**
   * Load running bundle routes from disk
   */
  async loadBundleRoutes(): Promise<
    Map<
      string,
      {
        bundleName: string;
        bundlePath: string;
        route: string;
        id: string;
        startTime: Date;
        isRunning: boolean;
      }
    >
  > {
    const bundleRoutes = new Map<
      string,
      {
        bundleName: string;
        bundlePath: string;
        route: string;
        id: string;
        startTime: Date;
        isRunning: boolean;
      }
    >();

    try {
      // Check if persistence file exists
      if (!fs.existsSync(this.persistenceFile)) {
        this.log(
          'yellow',
          'No persistence file found, starting with empty bundle routes',
        );
        return bundleRoutes;
      }

      const data = await fs.promises.readFile(this.persistenceFile, 'utf8');
      const serializedRoutes: PersistedBundleRoute[] = JSON.parse(data);

      // Convert back to Map and deserialize dates
      for (const route of serializedRoutes) {
        bundleRoutes.set(route.id, {
          bundleName: route.bundleName,
          bundlePath: route.bundlePath,
          route: route.route,
          id: route.id,
          startTime: new Date(route.startTime),
          isRunning: route.isRunning,
        });
      }

      this.log(
        'green',
        `Loaded ${bundleRoutes.size} bundle routes from persistence`,
      );
      return bundleRoutes;
    } catch (error) {
      this.log(
        'red',
        `Failed to load bundle routes: ${error instanceof Error ? error.message : 'Unknown error'}`,
      );
      // Return empty map on error rather than failing
      return bundleRoutes;
    }
  }

  /**
   * Clear all persisted bundle routes
   */
  async clearBundleRoutes(): Promise<void> {
    try {
      if (fs.existsSync(this.persistenceFile)) {
        await fs.promises.unlink(this.persistenceFile);
        this.log('yellow', 'Cleared all persisted bundle routes');
      }
    } catch (error) {
      this.log(
        'red',
        `Failed to clear bundle routes: ${error instanceof Error ? error.message : 'Unknown error'}`,
      );
      throw error;
    }
  }

  /**
   * Check if a bundle directory still exists on disk
   */
  bundleExists(bundlePath: string): boolean {
    try {
      return fs.existsSync(bundlePath) && fs.statSync(bundlePath).isDirectory();
    } catch {
      return false;
    }
  }

  /**
   * Validate and filter persisted bundles to only include those that still exist
   */
  async validateAndFilterBundles(
    bundleRoutes: Map<
      string,
      {
        bundleName: string;
        bundlePath: string;
        route: string;
        id: string;
        startTime: Date;
        isRunning: boolean;
      }
    >,
  ): Promise<{
    valid: Map<
      string,
      {
        bundleName: string;
        bundlePath: string;
        route: string;
        id: string;
        startTime: Date;
        isRunning: boolean;
      }
    >;
    removed: string[];
  }> {
    const validRoutes = new Map<
      string,
      {
        bundleName: string;
        bundlePath: string;
        route: string;
        id: string;
        startTime: Date;
        isRunning: boolean;
      }
    >();
    const removedBundles: string[] = [];

    for (const [id, bundle] of bundleRoutes.entries()) {
      if (this.bundleExists(bundle.bundlePath)) {
        validRoutes.set(id, bundle);
      } else {
        removedBundles.push(bundle.bundleName);
        this.log(
          'yellow',
          `Bundle ${bundle.bundleName} no longer exists at ${bundle.bundlePath}, removing from persistence`,
        );
      }
    }

    if (removedBundles.length > 0) {
      // Save the filtered routes back to persistence
      await this.saveBundleRoutes(validRoutes);
    }

    return {valid: validRoutes, removed: removedBundles};
  }
}
