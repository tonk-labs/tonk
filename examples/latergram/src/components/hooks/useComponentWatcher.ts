import { useEffect, useCallback, useRef } from 'react';
import { getVFSService } from '../../services/vfs-service';
import { componentRegistry } from '../ComponentRegistry';
import { buildAvailablePackages } from '../contextBuilder';
import { DocumentData } from '@tonk/core';
import { bytesToString } from '../../utils/vfs-utils';

export interface ComponentWatcherHook {
  watchComponent: (componentId: string, filePath: string) => Promise<void>;
  unwatchComponent: (componentId: string) => Promise<void>;
  compileAndUpdate: (componentId: string, code: string) => Promise<void>;
}

export function useComponentWatcher(): ComponentWatcherHook {
  const vfs = getVFSService();
  const watchersRef = useRef<Map<string, string>>(new Map());

  const compileAndUpdate = useCallback(
    async (componentId: string, code: string) => {
      try {
        const ts = (window as any).ts;
        if (!ts) {
          throw new Error('TypeScript compiler not loaded');
        }

        const compiled = ts.transpileModule(code, {
          compilerOptions: {
            jsx: ts.JsxEmit.React,
            module: ts.ModuleKind.CommonJS,
            target: ts.ScriptTarget.ES2020,
          },
        });

        // Always get fresh context to ensure all available components are included
        const freshPackages = buildAvailablePackages();
        const contextKeys = Object.keys(freshPackages);
        const contextValues = Object.values(freshPackages);

        const moduleFactory = new Function(
          ...contextKeys,
          `
        const exports = {};
        const module = { exports };

        ${compiled.outputText}

        return module.exports.default || module.exports;
      `
        );

        const component = moduleFactory(...contextValues);

        componentRegistry.update(componentId, component, 'success');
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : 'Compilation failed';
        console.error('Component compilation failed:', errorMessage);

        const ErrorComponent = () => {
          const React = (window as any).React;
          return React.createElement(
            'div',
            {
              style: {
                padding: '20px',
                backgroundColor: '#fee',
                border: '1px solid #fcc',
                borderRadius: '4px',
                color: '#c00',
              },
            },
            [
              React.createElement('h3', { key: 'title' }, 'Compilation Error'),
              React.createElement(
                'pre',
                {
                  key: 'error',
                  style: { marginTop: '10px', fontSize: '12px' },
                },
                errorMessage
              ),
            ]
          );
        };

        componentRegistry.update(
          componentId,
          ErrorComponent,
          'error',
          errorMessage
        );
      }
    },
    []
  );

  const watchComponent = useCallback(
    async (componentId: string, filePath: string) => {
      if (!vfs.isInitialized()) {
        console.error('VFS not initialized for watching');
        return;
      }

      try {
        const watchId = await vfs.watchFile(
          filePath,
          (content: DocumentData) => {
            const codeString = bytesToString(content);
            compileAndUpdate(componentId, codeString);
          }
        );

        watchersRef.current.set(componentId, watchId);

        try {
          const initialContent = await vfs.readBytesAsString(filePath);
          await compileAndUpdate(componentId, initialContent);
        } catch (err) {
          console.warn(
            'Could not load initial content for component:',
            componentId
          );
        }
      } catch (error) {
        console.error('Failed to watch component file:', error);
        componentRegistry.updateMetadata(componentId, {
          status: 'error',
          error: 'Failed to watch file',
        });
      }
    },
    [vfs, compileAndUpdate]
  );

  const unwatchComponent = useCallback(
    async (componentId: string) => {
      const watchId = watchersRef.current.get(componentId);
      if (watchId) {
        try {
          await vfs.unwatchFile(watchId);
          watchersRef.current.delete(componentId);
        } catch (error) {
          console.error('Failed to unwatch component:', error);
        }
      }
    },
    [vfs]
  );

  useEffect(() => {
    return () => {
      watchersRef.current.forEach(async watchId => {
        try {
          await vfs.unwatchFile(watchId);
        } catch (error) {
          console.error('Failed to cleanup watcher:', error);
        }
      });
      watchersRef.current.clear();
    };
  }, [vfs]);

  return {
    watchComponent,
    unwatchComponent,
    compileAndUpdate,
  };
}
