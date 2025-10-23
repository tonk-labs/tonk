import { useEffect, useCallback } from 'preact/hooks';
import { useTonk } from '../context/TonkContext';
import { useServiceWorker } from './useServiceWorker';
import { ALLOWED_ORIGINS } from '../constants';

const MAX_FILE_SIZE = 50 * 1024 * 1024; // 50MB

export function useDragDrop() {
  const { showError } = useTonk();
  const { processTonkFile } = useServiceWorker();

  // Native drag and drop
  const handleDrop = useCallback(
    async (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();

      const dt = e.dataTransfer;
      if (!dt) return;

      const files = dt.files;
      if (files.length === 0) return;

      // Filter for .tonk files
      const tonkFiles = Array.from(files).filter(file =>
        file.name.toLowerCase().endsWith('.tonk')
      );

      if (tonkFiles.length === 0) {
        showError('Please drop only .tonk files');
        return;
      }

      // Process the first .tonk file
      if (tonkFiles.length > 0) {
        await processTonkFile(tonkFiles[0]);
      }

      // Remove drag styling
      document.querySelectorAll('.boot-menu').forEach((menu: Element) => {
        const el = menu as HTMLElement;
        el.style.borderColor = '';
        el.style.background = '';
      });
    },
    [showError, processTonkFile]
  );

  const handleDragOver = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    const activeMenu = document.querySelector<HTMLElement>('.boot-menu');
    if (activeMenu) {
      activeMenu.classList.add('drag-over');
    }
  }, []);

  const handleDragLeave = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    document.querySelectorAll('.boot-menu').forEach((menu: Element) => {
      menu.classList.remove('drag-over');
    });
  }, []);

  // Cross-origin drag and drop via postMessage
  const handleCrossOriginMessage = useCallback(
    async (event: MessageEvent) => {
      // Validate origin
      if (!ALLOWED_ORIGINS.includes(event.origin)) {
        console.warn(
          '[Cross-Origin D&D] Rejected message from unauthorized origin:',
          event.origin
        );
        return;
      }

      const { type, fileName, fileData } = event.data;

      // Only handle tonk: prefixed messages
      if (!type || !type.startsWith('tonk:')) {
        return;
      }

      console.log('[Cross-Origin D&D] Received message:', type);

      switch (type) {
        case 'tonk:dragEnter': {
          const activeMenu = document.querySelector<HTMLElement>('.boot-menu');
          if (activeMenu) {
            activeMenu.classList.add('drag-over');
          }
          break;
        }

        case 'tonk:dragLeave': {
          document.querySelectorAll('.boot-menu').forEach((menu: Element) => {
            menu.classList.remove('drag-over');
          });
          break;
        }

        case 'tonk:drop': {
          // Remove drag styling
          document.querySelectorAll('.boot-menu').forEach((menu: Element) => {
            menu.classList.remove('drag-over');
          });

          // Validate file
          if (!fileName || typeof fileName !== 'string') {
            showError('Invalid file name received');
            return;
          }

          if (!fileName.toLowerCase().endsWith('.tonk')) {
            showError('Please drop only .tonk files');
            return;
          }

          if (!fileData || !(fileData instanceof ArrayBuffer)) {
            showError('Invalid file data received');
            return;
          }

          if (fileData.byteLength > MAX_FILE_SIZE) {
            showError(
              `File too large. Maximum size is ${MAX_FILE_SIZE / 1024 / 1024}MB`
            );
            return;
          }

          if (fileData.byteLength === 0) {
            showError('File is empty');
            return;
          }

          try {
            // Create File object to reuse processTonkFile
            const file = new File([fileData], fileName, {
              type: 'application/octet-stream',
              lastModified: Date.now(),
            });

            await processTonkFile(file);

            // Send success response to parent
            if (window.parent !== window) {
              ALLOWED_ORIGINS.forEach(origin => {
                try {
                  window.parent.postMessage(
                    {
                      type: 'tonk:dropResponse',
                      status: 'success',
                      fileName,
                    },
                    origin
                  );
                } catch (err) {
                  console.warn(
                    '[Cross-Origin D&D] Failed to send success response to parent:',
                    err
                  );
                  // Ignore errors for wrong origins
                }
              });
            }
          } catch (error: any) {
            showError(
              'Failed to process file: ' + (error.message || 'Unknown error')
            );

            // Send error response to parent
            if (window.parent !== window) {
              ALLOWED_ORIGINS.forEach(origin => {
                try {
                  window.parent.postMessage(
                    {
                      type: 'tonk:dropResponse',
                      status: 'error',
                      fileName,
                      error: error.message || 'Unknown error',
                    },
                    origin
                  );
                } catch (err) {
                  console.warn(
                    '[Cross-Origin D&D] Failed to send error response to parent:',
                    err
                  );
                  // Ignore
                }
              });
            }
          }
          break;
        }
      }
    },
    [showError, processTonkFile]
  );

  useEffect(() => {
    // Set up native drag and drop
    const preventDefaults = (e: Event) => {
      e.preventDefault();
      e.stopPropagation();
    };

    ['dragenter', 'dragover', 'dragleave', 'drop'].forEach(eventName => {
      document.body.addEventListener(eventName, preventDefaults, false);
    });

    document.body.addEventListener('dragover', handleDragOver as any, false);
    document.body.addEventListener('dragleave', handleDragLeave as any, false);
    document.body.addEventListener('drop', handleDrop as any, false);

    // Set up cross-origin drag and drop
    window.addEventListener('message', handleCrossOriginMessage);

    return () => {
      ['dragenter', 'dragover', 'dragleave', 'drop'].forEach(eventName => {
        document.body.removeEventListener(eventName, preventDefaults, false);
      });

      document.body.removeEventListener(
        'dragover',
        handleDragOver as any,
        false
      );
      document.body.removeEventListener(
        'dragleave',
        handleDragLeave as any,
        false
      );
      document.body.removeEventListener('drop', handleDrop as any, false);
      window.removeEventListener('message', handleCrossOriginMessage);
    };
  }, [handleDrop, handleDragOver, handleDragLeave, handleCrossOriginMessage]);
}
