import {
  HTMLContainer,
  Rectangle2d,
  ShapeUtil,
  T,
  type TLResizeInfo,
  type TLShapePartial,
  useIsDarkMode,
} from 'tldraw';
import { cn } from '@/lib/utils';
import { useFeatureFlagStore } from '../../../lib/featureFlags';
import { useThumbnail } from '../hooks/useThumbnail';
import { getAppHandler, getFileIcon } from '../utils/mimeResolver';
import { navigate } from '../utils/navigationHandler';
import type { FileIconShape } from './types';

export class FileIconUtil extends ShapeUtil<FileIconShape> {
  static override type = 'file-icon' as const;

  static override props = {
    filePath: T.string,
    fileName: T.string,
    mimeType: T.string,
    // Use .nullable().optional() to handle JSON round-trip (undefined -> null)
    customIcon: T.string.nullable().optional(),
    thumbnail: T.string.nullable().optional(),
    thumbnailPath: T.string.nullable().optional(),
    thumbnailVersion: T.number.nullable().optional(),
    appHandler: T.string.nullable().optional(),
    w: T.number,
    h: T.number,
  };

  override isAspectRatioLocked = () => false;
  override canResize = () => {
    // Disable resizing when minimal desktop UI is enabled
    const { flags } = useFeatureFlagStore.getState();
    return !flags.minimalDesktopUI;
  };
  override canBind = () => false;

  getDefaultProps(): FileIconShape['props'] {
    return {
      filePath: '',
      fileName: 'Untitled',
      mimeType: 'application/octet-stream',
      w: 90,
      h: 110,
    };
  }

  getGeometry(shape: FileIconShape) {
    return new Rectangle2d({
      width: shape.props.w,
      height: shape.props.h,
      isFilled: true,
    });
  }

  component(shape: FileIconShape) {
    const { thumbnailPath, customIcon, mimeType } = shape.props;

    // Load thumbnail from VFS path
    const { thumbnail } = useThumbnail(thumbnailPath);

    const icon = customIcon || getFileIcon(mimeType);
    const hasThumbnail = !!thumbnail;

    // Use tldraw's hook for reactive dark mode detection
    const isDark = useIsDarkMode();

    // Detect selected state
    const isSelected = this.editor.getSelectedShapeIds().includes(shape.id);

    return (
      <HTMLContainer
        className={cn(
          'flex flex-col items-center p-1 border-2 bg-transparent border-solid border-transparent transition-all duration-300 ease-in-out overflow-visible select-none translate-y-0',
          isSelected && 'bg-blue-400 text-white',
          'group'
        )}
        style={{
          width: shape.props.w,
          height: shape.props.h,
        }}
      >
        <div
          className={
            'font-[40px] mb-2 [filter: drop-shadow(0 4px 3px rgba(0, 0, 0, 0.14))]'
          }
        >
          {hasThumbnail ? (
            <img
              src={thumbnail}
              alt={shape.props.fileName}
              style={{
                width: '100%',
                height: '100%',
                objectFit: 'contain',
                borderRadius: '4px',
              }}
            />
          ) : (
            icon
          )}
        </div>
        <div
          className={
            'text-[0.5rem] text-center font-medium text-balance pointer-events-none select-none w-[200%] -mt-1'
          }
        >
          <mark
            className={cn(
              'p-0.5 rounded-[1px] z-1000',
              !isSelected && 'bg-transparent',
              isDark ? 'text-white' : 'text-black',
              isSelected ? 'bg-blue-400 z-10000000 text-white' : ''
            )}
          >
            {shape.props.fileName}
          </mark>
        </div>
      </HTMLContainer>
    );
  }

  indicator(shape: FileIconShape) {
    return (
      <rect
        width={shape.props.w}
        height={shape.props.h}
        rx={0}
        ry={0}
        fill="transparent"
        stroke="#0078d433"
        strokeWidth={1}
      />
    );
  }

  override onResize(
    _shape: FileIconShape,
    info: TLResizeInfo<FileIconShape>
  ): Omit<TLShapePartial<FileIconShape>, 'id' | 'type'> | undefined {
    // Calculate new dimensions with minimum constraints
    const newWidth = info.scaleX * info.initialShape.props.w;
    const newHeight = info.scaleY * info.initialShape.props.h;

    // Validate dimensions are positive and finite
    if (
      !Number.isFinite(newWidth) ||
      !Number.isFinite(newHeight) ||
      newWidth <= 0 ||
      newHeight <= 0
    ) {
      console.warn('[FileIconUtil] Invalid resize dimensions, ignoring');
      return undefined;
    }

    return {
      props: {
        w: Math.max(70, newWidth),
        h: Math.max(90, newHeight),
      },
    };
  }

  override onDoubleClick(shape: FileIconShape): void {
    const { filePath, mimeType, appHandler } = shape.props;

    // Validate file path is not empty
    if (!filePath || filePath.trim().length === 0) {
      console.error('[FileIconUtil] Cannot open file: empty file path');
      return;
    }

    const targetApp = getAppHandler(mimeType, appHandler);
    const encodedPath = encodeURIComponent(filePath);
    navigate(`/${targetApp}?file=${encodedPath}`);
  }
}
