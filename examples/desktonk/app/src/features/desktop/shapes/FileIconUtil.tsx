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
import { getVFSService } from '../../../lib/vfs-service';
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

    // Build class name - TLDraw handles dragging visual feedback internally
    const className = isSelected ? 'file-icon selected' : 'file-icon';

    const handleLabelDoubleClick = async (e: React.MouseEvent) => {
      e.stopPropagation();
      
      const vfs = getVFSService();
      const { filePath, fileName } = shape.props;
      
      const newName = prompt('Rename file', fileName)?.trim();
      if (!newName || newName === fileName) return;

      if (/[\\/]/.test(newName)) {
        alert('File name cannot contain / or \\');
        return;
      }

      const dir = filePath.slice(0, filePath.lastIndexOf('/') + 1);
      const newPath = dir + newName;
      
      try {
        await vfs.renameFile(filePath, newPath);
        this.editor.updateShape({
          id: shape.id,
          type: 'file-icon',
          props: { fileName: newName, filePath: newPath },
        });
      } catch (err) {
        console.error('Rename failed', err);
        alert('Failed to rename file');
      }
    };

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
        <div 
          className="file-icon-label"
          onDoubleClick={handleLabelDoubleClick}
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
        rx={24}
        ry={24}
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
    if (!isFinite(newWidth) || !isFinite(newHeight) || newWidth <= 0 || newHeight <= 0) {
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
