import { Plugin } from 'vite';
import { build } from 'vite';
import path from 'path';
import fs from 'fs';

interface ViteServiceWorkerOptions {
  /**
   * Service worker entry point
   */
  entry: string;
  /**
   * Output filename for the service worker
   */
  filename?: string;
  /**
   * Include Vite runtime in the service worker bundle
   */
  includeViteRuntime?: boolean;
  /**
   * Additional Vite config for service worker build
   */
  viteConfig?: any;
}

/**
 * Vite plugin for building TypeScript service workers with Vite runtime capabilities
 */
export function viteServiceWorker(options: ViteServiceWorkerOptions): Plugin {
  const {
    entry,
    filename = 'vite-compiler-sw.js',
    includeViteRuntime = true,
    viteConfig = {},
  } = options;

  return {
    name: 'vite-service-worker',
    enforce: 'post',

    async generateBundle(outputOptions, bundle) {
      // Build the service worker as a separate Vite build
      const swBuildConfig = {
        configFile: false,
        build: {
          lib: {
            entry,
            name: 'ServiceWorker',
            formats: ['iife'] as const,
            fileName: () => filename.replace('.js', ''),
          },
          outDir: 'dist-sw',
          rollupOptions: {
            output: {
              entryFileNames: filename,
              format: 'iife',
            },
            external: [], // Bundle everything for service worker
          },
          sourcemap: true,
          minify: false, // Keep readable for debugging
          target: 'esnext',
        },
        define: {
          'process.env.NODE_ENV': JSON.stringify('production'),
          'import.meta.env.DEV': 'false',
          'import.meta.env.PROD': 'true',
        },
        ...viteConfig,
      };

      try {
        console.log('[Vite SW Plugin] Building service worker...');

        // Build the service worker
        await build(swBuildConfig);

        // Read the built service worker
        const swPath = path.join(process.cwd(), 'dist-sw', filename);

        if (fs.existsSync(swPath)) {
          const swContent = fs.readFileSync(swPath, 'utf-8');

          // Add the service worker to the main bundle
          this.emitFile({
            type: 'asset',
            fileName: filename,
            source: swContent,
          });

          console.log(`[Vite SW Plugin] Service worker built: ${filename}`);

          // Clean up temporary build directory
          fs.rmSync(path.join(process.cwd(), 'dist-sw'), {
            recursive: true,
            force: true,
          });
        } else {
          console.error(
            '[Vite SW Plugin] Service worker build failed - file not found'
          );
        }
      } catch (error) {
        console.error('[Vite SW Plugin] Service worker build error:', error);
        throw error;
      }
    },

    configureServer(server) {
      // Serve the service worker during development
      server.middlewares.use(`/${filename}`, async (req, res, next) => {
        try {
          const swPath = path.resolve(process.cwd(), entry);

          if (!fs.existsSync(swPath)) {
            res.statusCode = 404;
            res.end('Service worker not found');
            return;
          }

          // For development, we'll transpile on the fly
          const result = await server.ssrTransform(
            fs.readFileSync(swPath, 'utf-8'),
            null,
            swPath
          );

          res.setHeader('Content-Type', 'application/javascript');
          res.setHeader('Service-Worker-Allowed', '/');
          res.end(result.code);
        } catch (error) {
          console.error('[Vite SW Plugin] Dev server error:', error);
          next(error);
        }
      });
    },
  };
}

