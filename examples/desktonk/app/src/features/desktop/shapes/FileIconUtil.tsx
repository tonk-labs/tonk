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
import './fileIcon.css';

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
    const icon = shape.props.customIcon || getFileIcon(shape.props.mimeType);

    // Detect selected state
    const isSelected = this.editor.getSelectedShapeIds().includes(shape.id);

    // Detect dragging state - check if the current tool is 'select' and shape is being moved
    // In TLDraw v4, we can approximate dragging by checking if the shape is selected
    // The dragging visual state will be applied when the user starts moving the shape
    const isDragging = false; // TLDraw handles dragging visual feedback internally

    // Build class name
    const className = [
      'file-icon',
      isSelected ? 'selected' : '',
      isDragging ? 'dragging' : '',
    ].filter(Boolean).join(' ');

    return (
      <HTMLContainer
        className={className}
        style={{
          width: shape.props.w,
          height: shape.props.h,
        }}
      >
        <div className="file-icon-emoji">
          {icon}
        </div>
        <div className="file-icon-label">
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
        rx={12}
        ry={12}
        fill="transparent"
        stroke="#0078d4"
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
        w: Math.max(70, info.scaleX * info.initialShape.props.w),
        h: Math.max(90, info.scaleY * info.initialShape.props.h),
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
