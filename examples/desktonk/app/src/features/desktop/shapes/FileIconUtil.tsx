import {
  HTMLContainer,
  Rectangle2d,
  ShapeUtil,
  type TLResizeInfo,
  type TLShapePartial,
  T,
} from 'tldraw';
import type { FileIconShape } from './types';
import { getFileIcon, getAppHandler } from '../utils/mimeResolver';
import { navigate } from '../utils/navigationHandler';

export class FileIconUtil extends ShapeUtil<FileIconShape> {
  static override type = 'file-icon' as const;

  static override props = {
    filePath: T.string,
    fileName: T.string,
    mimeType: T.string,
    customIcon: T.optional(T.string),
    appHandler: T.optional(T.string),
    w: T.number,
    h: T.number,
  };

  override isAspectRatioLocked = () => false;
  override canResize = () => true;
  override canBind = () => false;

  getDefaultProps(): FileIconShape['props'] {
    return {
      filePath: '',
      fileName: 'Untitled',
      mimeType: 'application/octet-stream',
      w: 80,
      h: 100,
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
    const icon = shape.props.customIcon || getFileIcon(shape.props.mimeType);

    return (
      <HTMLContainer
        style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          pointerEvents: 'all',
          backgroundColor: '#2d2d2d',
          border: '2px solid #444',
          borderRadius: '8px',
          padding: '8px',
          width: shape.props.w,
          height: shape.props.h,
        }}
      >
        <div
          style={{
            fontSize: '32px',
            marginBottom: '8px',
          }}
        >
          {icon}
        </div>
        <div
          style={{
            fontSize: '12px',
            color: '#fff',
            textAlign: 'center',
            wordBreak: 'break-word',
            maxWidth: '100%',
          }}
        >
          {shape.props.fileName}
        </div>
      </HTMLContainer>
    );
  }

  indicator(shape: FileIconShape) {
    return (
      <rect
        width={shape.props.w}
        height={shape.props.h}
        fill="transparent"
        stroke="blue"
        strokeWidth={2}
      />
    );
  }

  override onResize(
    _shape: FileIconShape,
    info: TLResizeInfo<FileIconShape>
  ): Omit<TLShapePartial<FileIconShape>, 'id' | 'type'> | undefined {
    return {
      props: {
        w: Math.max(60, info.scaleX * info.initialShape.props.w),
        h: Math.max(80, info.scaleY * info.initialShape.props.h),
      },
    };
  }

  override onDoubleClick(shape: FileIconShape): void {
    const { filePath, mimeType, appHandler } = shape.props;
    const targetApp = getAppHandler(mimeType, appHandler);
    const encodedPath = encodeURIComponent(filePath);
    navigate(`/${targetApp}?file=${encodedPath}`);
  }
}
