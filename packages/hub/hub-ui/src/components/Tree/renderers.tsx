import React from "react";
import { TreeRenderProps, TreeItem } from "react-complex-tree";
import { File, Folder, Cylinder, Package, FileDigit } from "lucide-react";
import styles from "./Tree.module.css";
import { TreeItemMetadata, FileType } from "./index";

const cx = (...classNames: Array<string | undefined | false>) =>
  classNames.filter((cn) => !!cn).join(" ");

const renderItem = (
  item: TreeItem<TreeItemMetadata>,
  title: React.ReactNode
) => {
  switch (item.data.fileType) {
    case FileType.App: {
      return (
        <span className={styles.treeNodeFileSpan}>
          <Folder size={16} />
          {title}
        </span>
      );
    }
    case FileType.Store: {
      return (
        <span className={styles.treeNodeFileSpan}>
          <Cylinder size={16} />
          {title}
        </span>
      );
    }
    case FileType.Integration: {
      return (
        <span className={styles.treeNodeFileSpan}>
          <Package size={16} />
          {title}
        </span>
      );
    }
    case FileType.Data: {
      return (
        <span className={styles.treeNodeFileSpan}>
          <FileDigit size={16} />
          {title}
        </span>
      );
    }
    default: {
      return (
        <span className={styles.treeNodeFileSpan}>
          <File size={16} />
          {title}
        </span>
      );
    }
  }
};

export const renderers: TreeRenderProps<TreeItemMetadata> = {
  renderItem: ({ item, depth, children, title, context, arrow }) => {
    const InteractiveComponent = context.isRenaming ? "div" : "button";
    const type = context.isRenaming ? undefined : "button";
    // TODO have only root li component create all the classes
    return (
      <li
        {...(context.itemContainerWithChildrenProps as any)}
        className={cx(
          "rct-tree-item-li",
          item.isFolder && "rct-tree-item-li-isFolder",
          context.isSelected && "rct-tree-item-li-selected",
          context.isExpanded && "rct-tree-item-li-expanded",
          context.isFocused && "rct-tree-item-li-focused",
          context.isDraggingOver && "rct-tree-item-li-dragging-over",
          context.isSearchMatching && "rct-tree-item-li-search-match"
        )}
      >
        <div
          {...(context.itemContainerWithoutChildrenProps as any)}
          style={{ "--depthOffset": `${(depth + 1) * 10}px` }}
          className={cx(
            "rct-tree-item-title-container",
            item.isFolder && "rct-tree-item-title-container-isFolder",
            context.isSelected && "rct-tree-item-title-container-selected",
            context.isExpanded && "rct-tree-item-title-container-expanded",
            context.isFocused && "rct-tree-item-title-container-focused",
            context.isDraggingOver &&
              "rct-tree-item-title-container-dragging-over",
            context.isSearchMatching &&
              "rct-tree-item-title-container-search-match"
          )}
        >
          {arrow}
          <InteractiveComponent
            type={type}
            {...(context.interactiveElementProps as any)}
            className={cx(
              "rct-tree-item-button",
              item.isFolder && "rct-tree-item-button-isFolder",
              context.isSelected && "rct-tree-item-button-selected",
              context.isExpanded && "rct-tree-item-button-expanded",
              context.isFocused && "rct-tree-item-button-focused",
              context.isDraggingOver && "rct-tree-item-button-dragging-over",
              context.isSearchMatching && "rct-tree-item-button-search-match"
            )}
          >
            {item.isFolder ? (
              item.data.fileType === FileType.Section ? (
                <span className={styles.treeSection}>{title}</span>
              ) : (
                title
              )
            ) : (
              renderItem(item, title)
            )}
          </InteractiveComponent>
        </div>
        {children}
      </li>
    );
  },
};
