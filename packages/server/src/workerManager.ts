import path from 'path';
import fs from 'fs';
import worker from './workers.js';
import {logger} from './logger.js';
/**
 * Load all integrations from the integrations directory
 */
export const loadIntegrations = (): void => {
  try {
    const integrationsPath = path.join(process.cwd(), 'integrations');
    const packageJsonPath = path.join(integrationsPath, 'package.json');

    logger.info('WHAT ARE THE PATHS???', packageJsonPath);
    if (!fs.existsSync(packageJsonPath)) {
      console.warn('No package.json found in integrations directory');
      return;
    }

    const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf-8'));
    const dependencies = packageJson.dependencies || {};

    // Load each integration's config
    for (const packageName of Object.keys(dependencies)) {
      const configPath = path.join(
        integrationsPath,
        'node_modules',
        packageName,
        'tonk.config.json',
      );

      if (fs.existsSync(configPath)) {
        try {
          const config = JSON.parse(fs.readFileSync(configPath, 'utf-8'));
          logger.info('Registering integration', config);
          worker.registerIntegration({
            name: packageName,
            ...config,
          });
        } catch (error) {
          console.error(`Error loading config for ${packageName}:`, error);
        }
      } else {
        console.warn(`No config.json found for integration ${packageName}`);
      }
    }
  } catch (error) {
    console.error('Error loading integrations:', error);
  }
};
