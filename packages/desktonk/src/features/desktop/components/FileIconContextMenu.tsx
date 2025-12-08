import { getVFSService } from '@/vfs-client';
import {
  DefaultContextMenu,
  DefaultContextMenuContent,
  TldrawUiMenuGroup,
  TldrawUiMenuItem,
  useEditor,
} from 'tldraw';
import type { FileIconShape } from '../shapes/types';

export function FileIconContextMenu(props: {
  // tldraw doesn't export context menu props type, so we use unknown to avoid explicit any
  // biome-ignore lint/suspicious/noExplicitAny: Type not exported by tldraw
  [key: string]: any;
}) {
  const editor = useEditor();

  // Check if only file-icon shapes are selected
  const selectedShapeIds = editor.getSelectedShapeIds();
  const selectedShapes = Array.from(selectedShapeIds)
    .map(id => editor.getShape(id))
    .filter(Boolean); // Remove any null/undefined shapes
  const onlyFileIcons =
    selectedShapes.length > 0 &&
    selectedShapes.every(s => s?.type === 'file-icon');
  const singleFileIcon = onlyFileIcons && selectedShapes.length === 1;

  const handleRename = () => {
    if (!singleFileIcon) return;

    const shape = selectedShapes[0] as FileIconShape;
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

    vfs
      .renameFile(filePath, newPath)
      .then(() => {
        editor.updateShape({
          id: shape.id,
          type: 'file-icon',
          props: { fileName: newName, filePath: newPath },
        });
      })
      .catch(err => {
        console.error('Rename failed', err);
        alert('Failed to rename file');
      });
  };

  const handleDelete = async () => {
    if (!onlyFileIcons) return;

    const vfs = getVFSService();
    const fileShapes = selectedShapes as FileIconShape[];

    const confirmMsg =
      fileShapes.length === 1
        ? `Delete ${fileShapes[0].props.fileName}?`
        : `Delete ${fileShapes.length} files?`;

    if (!confirm(confirmMsg)) return;

    // Delete from VFS
    for (const shape of fileShapes) {
      try {
        await vfs.deleteFile(shape.props.filePath);
      } catch (err) {
        console.error('Failed to delete file', err);
      }
    }

    // Delete shapes from canvas
    editor.deleteShapes(fileShapes.map(s => s.id));
  };

  const handleCopy = () => {
    if (!onlyFileIcons) return;

    // For now, just copy the file paths to clipboard
    const filePaths = (selectedShapes as FileIconShape[])
      .map(s => s.props.filePath)
      .join('\n');

    navigator.clipboard
      .writeText(filePaths)
      .then(() => console.log('Copied file paths'))
      .catch(err => console.error('Failed to copy', err));
  };

  const handleCut = async () => {
    if (!onlyFileIcons) return;

    // Copy then delete
    handleCopy();

    // Store file paths for paste operation
    const filePaths = (selectedShapes as FileIconShape[]).map(
      s => s.props.filePath
    );
    sessionStorage.setItem('cutFiles', JSON.stringify(filePaths));

    // Visual feedback - could fade out the icons
    console.log('Cut files:', filePaths);
  };

  return (
    <DefaultContextMenu {...props}>
      {onlyFileIcons && (
        <TldrawUiMenuGroup id="file-operations">
          {singleFileIcon && (
            <TldrawUiMenuItem
              id="rename"
              label="Rename"
              icon="edit"
              onSelect={handleRename}
            />
          )}
          <TldrawUiMenuItem
            id="cut-file"
            label="Cut"
            kbd="$X"
            onSelect={handleCut}
          />
          <TldrawUiMenuItem
            id="copy-file"
            label="Copy"
            kbd="$C"
            onSelect={handleCopy}
          />
          <TldrawUiMenuItem
            id="delete-file"
            label="Delete"
            kbd="⌫"
            onSelect={handleDelete}
          />
        </TldrawUiMenuGroup>
      )}

      {!onlyFileIcons && <DefaultContextMenuContent />}
    </DefaultContextMenu>
  );
}
