import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { Tldraw, track, useEditor, useToasts } from 'tldraw';
import 'tldraw/tldraw.css';
import type { TLShapeId } from 'tldraw';
import { getVFSService } from '@/vfs-client';
import { useFeatureFlag } from '../../../hooks/useFeatureFlag';
import { useCanvasPersistence } from '../hooks';
import { useDesktop, useDesktopActions } from '../hooks/useDesktop';
import { useFileDrop } from '../hooks/useFileDrop';
import { getDesktopService } from '../services/DesktopService';
import { FileIconUtil } from '../shapes';
import { setNavigationHandler } from '../utils/navigationHandler';
import styles from './desktop.module.css';
import { FeatureFlagMenu } from './FeatureFlagMenu';
import { FileIconContextMenu } from './FileIconContextMenu';

const customShapeUtils = [FileIconUtil];

const DesktopInner = track(() => {
  const navigate = useNavigate();
  const { addToast } = useToasts();
  const editor = useEditor();

  // Enable canvas state persistence (must be ready before desktop initialization)
  const { isReady: canvasPersistenceReady } = useCanvasPersistence();

  // Get desktop state from service
  const { files, positions, isLoading } = useDesktop();
  const { setPosition } = useDesktopActions();

  // Initialize desktop service when canvas persistence is ready
  useEffect(() => {
    if (!canvasPersistenceReady) return;

    const service = getDesktopService();

    service.initialize().catch(error => {
      console.error('[Desktop] Failed to initialize service:', error);
      addToast({
        title: 'Failed to load desktop',
        severity: 'error',
      });
    });
  }, [canvasPersistenceReady, addToast]);

  // Listen for theme changes from parent window (via Router.tsx)
  // This is separate from service init so it works immediately without waiting for VFS
  useEffect(() => {
    const handleThemeChange = (e: CustomEvent<{ isDark: boolean }>) => {
      editor.user.updateUserPreferences({
        colorScheme: e.detail.isDark ? 'dark' : 'light',
      });
    };
    window.addEventListener(
      'theme-changed',
      handleThemeChange as EventListener
    );

    return () => {
      window.removeEventListener(
        'theme-changed',
        handleThemeChange as EventListener
      );
    };
  }, [editor.user]);

  // Sync files to TLDraw shapes
  useEffect(() => {
    if (isLoading || !canvasPersistenceReady) return;

    console.log('[Desktop] Syncing', files.length, 'files to canvas');

    // Get current shapes
    const existingShapes = new Map(
      Array.from(editor.getCurrentPageShapeIds())
        .map(id => editor.getShape(id))
        .filter(
          (shape): shape is NonNullable<typeof shape> =>
            shape?.type === 'file-icon'
        )
        .map(shape => [shape.id, shape])
    );

    const currentFileIds = new Set<string>();

    // Create or update shapes for each file
    files.forEach(file => {
      const fileName = file.path.split('/').pop() || file.path;
      // Use filename with extension for dotfiles, without extension for others
      const fileId = fileName.startsWith('.')
        ? fileName
        : fileName.replace(/\.[^.]+$/, '');
      const shapeId = `shape:file-icon:${fileId}` as TLShapeId;
      currentFileIds.add(shapeId);

      const position = positions.get(fileId);

      if (!position) {
        console.warn('[Desktop] No position for file:', fileId);
        return;
      }

      const existingShape = existingShapes.get(shapeId);

      if (existingShape) {
        // Check if position or props changed
        // biome-ignore lint/suspicious/noExplicitAny: Shape props type
        const existingProps = (existingShape as any).props || {};
        const positionChanged =
          existingShape.x !== position.x || existingShape.y !== position.y;
        const propsChanged =
          existingProps.thumbnailPath !== file.desktopMeta?.thumbnailPath ||
          existingProps.thumbnailVersion !==
            file.desktopMeta?.thumbnailVersion ||
          existingProps.mimeType !== file.mimeType ||
          existingProps.customIcon !== file.desktopMeta?.icon;

        if (positionChanged || propsChanged) {
          console.log('[Desktop] Updating shape:', fileId, {
            positionChanged,
            propsChanged,
            thumbnailVersion: file.desktopMeta?.thumbnailVersion,
          });
          editor.updateShape({
            id: shapeId as unknown as TLShapeId,
            type: 'file-icon',
            x: position.x,
            y: position.y,
            props: {
              ...existingProps,
              thumbnailPath: file.desktopMeta?.thumbnailPath,
              thumbnailVersion: file.desktopMeta?.thumbnailVersion,
              mimeType: file.mimeType,
              customIcon: file.desktopMeta?.icon,
            },
          });
        }
        existingShapes.delete(shapeId);
      } else {
        // Create new shape
        console.log('[Desktop] Creating new shape:', fileId, position);
        try {
          editor.createShape({
            id: shapeId as unknown as TLShapeId,
            type: 'file-icon',
            x: position.x,
            y: position.y,
            props: {
              filePath: file.path,
              fileName: file.name,
              mimeType: file.mimeType,
              customIcon: file.desktopMeta?.icon,
              thumbnailPath: file.desktopMeta?.thumbnailPath,
              thumbnailVersion: file.desktopMeta?.thumbnailVersion,
              appHandler: file.desktopMeta?.appHandler,
              w: 80,
              h: 100,
            },
          });
        } catch (error) {
          console.error(
            '[Desktop] Failed to create shape for:',
            file.name,
            error
          );
        }
      }
    });

    // Delete shapes for files that no longer exist
    for (const [shapeId, shape] of existingShapes) {
      if (!currentFileIds.has(shapeId)) {
        console.log('[Desktop] Deleting shape for removed file:', shapeId);
        editor.deleteShape(shape.id);
      }
    }
  }, [files, positions, isLoading, editor, canvasPersistenceReady]);

  // Listen for shape position changes and deletions
  useEffect(() => {
    if (!canvasPersistenceReady) return;

    const service = getDesktopService();
    const vfs = getVFSService();

    const unsubscribe = editor.store.listen(
      change => {
        // Handle position updates
        const updatedShapes = [
          ...Object.values(change.changes.updated).map(([_prev, next]) => next),
        ];

        for (const shape of updatedShapes) {
          if (shape.typeName !== 'shape' || shape.type !== 'file-icon') {
            continue;
          }

          // Extract fileId from shape id (format: "shape:file-icon:{fileId}")
          const fileId = shape.id.replace('shape:file-icon:', '');

          // Save position to service (debounced internally)
          setPosition(fileId, shape.x, shape.y);
        }

        // Handle deletions
        const removedShapes = Object.values(change.changes.removed);
        for (const shape of removedShapes) {
          if (shape.typeName === 'shape' && shape.type === 'file-icon') {
            // biome-ignore lint/suspicious/noExplicitAny: Shape type casting
            const fileIconShape = shape as any;
            const fileId = shape.id.replace('shape:file-icon:', '');
            const filePath = fileIconShape.props?.filePath;

            if (filePath) {
              console.log(
                '[Desktop] Shape deleted, removing file from VFS:',
                filePath
              );
              // Delete from VFS
              vfs.deleteFile(filePath).catch(err => {
                console.error('[Desktop] Failed to delete file from VFS:', err);
              });
              // Delete position file
              service.onFileDeleted(fileId).catch(err => {
                console.error('[Desktop] Failed to delete position file:', err);
              });
            }
          }
        }
      },
      { source: 'user', scope: 'document' }
    );

    return unsubscribe;
  }, [editor, setPosition, canvasPersistenceReady]);

  // Set up navigation handler for double-click events
  useEffect(() => {
    setNavigationHandler((path: string) => {
      navigate(path);
    }, addToast);

    return () => {
      setNavigationHandler(null);
    };
  }, [navigate, addToast]);

  if (isLoading) {
    return <div></div>;
  }

  // Show empty state overlay when no files exist
  return (
    <>
      {!isLoading && files.length === 0 && (
        <div
          style={{
            position: 'absolute',
            top: '50%',
            left: '50%',
            transform: 'translate(-50%, -50%)',
            textAlign: 'center',
            color: '#666',
            fontSize: '1.25rem',
            zIndex: 1000,
            pointerEvents: 'none',
          }}
        >
          <div style={{ fontSize: '3rem', marginBottom: '1rem' }}>
            <img src={'tonk.svg'} alt="Tonk" />
          </div>
          <div>Welcome to your Desktonk</div>
          <div style={{ fontSize: '1rem', marginTop: '0.5rem', color: '#888' }}>
            Drop files into the desktop to get started
          </div>
        </div>
      )}
    </>
  );
});

function Desktop() {
  const minimalUI = useFeatureFlag('minimalDesktopUI');
  const lockCamera = useFeatureFlag('lockCamera');

  // Camera options based on feature flag
  const cameraOptions = lockCamera
    ? {
        isLocked: false,
        wheelBehavior: 'none' as const,
        panSpeed: 0,
        zoomSpeed: 0,
      }
    : {
        isLocked: false,
        wheelBehavior: 'zoom' as const,
        zoomSteps: [0.1, 0.25, 0.5, 0.75, 1, 1.5, 2, 2.5, 3, 4, 5, 8],
      };

  // Define minimal UI components
  const minimalComponents = {
    Toolbar: null,
    StylePanel: null,
    ActionsMenu: null,
    HelpMenu: null,
    MainMenu: FeatureFlagMenu,
    PageMenu: null,
    NavigationPanel: null,
    Minimap: null,
    QuickActions: null,
    HelperButtons: null,
    TopPanel: null,
    SharePanel: null,
    DebugPanel: null,
    ContextMenu: FileIconContextMenu,
  };

  // Default components
  const defaultComponents = {
    MainMenu: FeatureFlagMenu,
    ContextMenu: FileIconContextMenu,
  };

  const components = minimalUI ? minimalComponents : defaultComponents;

  const editorOptions = minimalUI
    ? {
        createTextOnCanvasDoubleClick: false,
      }
    : undefined;

  // biome-ignore lint/suspicious/noExplicitAny: Editor type is complex
  const handleMount = (editor: any) => {
    const zoomLevel = 1.35;
    editor.setCamera(
      { x: 0, y: 0, z: zoomLevel },
      { animation: { duration: 0 } }
    );

    // Set initial theme immediately on mount (before any useEffect runs)
    // This fixes the race condition where tldraw renders with default light theme
    // before canvasPersistenceReady becomes true
    const isDark =
      localStorage.theme === 'dark' ||
      (!('theme' in localStorage) &&
        window.matchMedia('(prefers-color-scheme: dark)').matches);
    editor.user.updateUserPreferences({
      colorScheme: isDark ? 'dark' : 'light',
    });
  };

  return (
    <div className={styles.desktopContainer}>
      <Tldraw
        key={`tldraw-${lockCamera}`}
        className={styles.tldrawContainer}
        shapeUtils={customShapeUtils}
        components={components}
        cameraOptions={cameraOptions}
        options={editorOptions}
        onMount={handleMount}
      >
        <DragDropWrapper />
      </Tldraw>
    </div>
  );
}

// Wrapper that connects drag handlers to outer container
function DragDropWrapper() {
  const {
    isDraggingOver,
    handleDrop,
    handleDragOver,
    handleDragEnter,
    handleDragLeave,
  } = useFileDrop();

  useEffect(() => {
    const container = document.querySelector(
      `.${styles.desktopContainer}`
    ) as HTMLElement;
    if (!container) return;

    const dropHandler = (e: DragEvent) => {
      // biome-ignore lint/suspicious/noExplicitAny: Drag event type mismatch
      handleDrop(e as any);
    };
    const dragOverHandler = (e: DragEvent) => {
      // biome-ignore lint/suspicious/noExplicitAny: Drag event type mismatch
      handleDragOver(e as any);
    };
    const dragEnterHandler = (e: DragEvent) => {
      // biome-ignore lint/suspicious/noExplicitAny: Drag event type mismatch
      handleDragEnter(e as any);
    };
    const dragLeaveHandler = (e: DragEvent) => {
      // biome-ignore lint/suspicious/noExplicitAny: Drag event type mismatch
      handleDragLeave(e as any);
    };

    container.addEventListener('drop', dropHandler, true);
    container.addEventListener('dragover', dragOverHandler, true);
    container.addEventListener('dragenter', dragEnterHandler, true);
    container.addEventListener('dragleave', dragLeaveHandler, true);

    if (isDraggingOver) {
      container.classList.add(styles.draggingOver);
    } else {
      container.classList.remove(styles.draggingOver);
    }

    return () => {
      container.removeEventListener('drop', dropHandler, true);
      container.removeEventListener('dragover', dragOverHandler, true);
      container.removeEventListener('dragenter', dragEnterHandler, true);
      container.removeEventListener('dragleave', dragLeaveHandler, true);
      container.classList.remove(styles.draggingOver);
    };
  }, [
    isDraggingOver,
    handleDrop,
    handleDragOver,
    handleDragEnter,
    handleDragLeave,
  ]);

  const dragOverlay = isDraggingOver ? (
    <div className={styles.dropOverlay}>
      <div className={styles.dropMessage}>
        <div className={styles.dropIcon}>📁</div>
        <div>Drop files to upload</div>
      </div>
    </div>
  ) : null;

  return (
    <>
      <DesktopInner />
      {dragOverlay}
    </>
  );
}

export default Desktop;
