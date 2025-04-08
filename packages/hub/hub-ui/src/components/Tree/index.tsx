import React, { useEffect, useMemo } from "react";
import {
  UncontrolledTreeEnvironment,
  StaticTreeDataProvider,
  Tree as ComplexTree,
  TreeItemIndex,
} from "react-complex-tree";
import "react-complex-tree/lib/style-modern.css";
import { renderers } from "./renderers";
import { useProjectStore } from "../../stores/projectStore";
import { change } from "@automerge/automerge";

export enum FileType {
  "Section",
  "App",
  "Store",
  "Integration",
  "Data",
}

export interface TreeItemMetadata {
  isSection?: boolean;
  fileType: FileType;
  name: string;
}

export interface TreeItem {
  index: string;
  isFolder?: boolean;
  children: string[];
  data: TreeItemMetadata;
}

export interface TreeItems {
  [key: string]: TreeItem;
}

interface TreeProps {
  treeId?: string;
  rootItem?: string;
  treeLabel?: string;
}

export const Tree: React.FC<TreeProps> = ({
  treeId = "tree-1",
  rootItem = "root",
  treeLabel = "Project Tree",
}) => {
  const {
    items: _items,
    loadProjects,
    isLoading,
    changedParents,
    setSelectedItem,
  } = useProjectStore();
  const items = useMemo(() => {
    return({ ..._items })}, [_items]);
  const dataProvider = useMemo(
    () =>
      new StaticTreeDataProvider(items, (item, newName) => ({
        ...item,
        data: {
          ...item.data,
          name: newName,
        },
      })),
    [items]
  );

  useEffect(() => {
    loadProjects();
  }, []);

  useEffect(() => {
    if (dataProvider) {
      // Force a complete re-render of the tree by emitting changes for the root
      dataProvider.onDidChangeTreeDataEmitter.emit(changedParents);
    }
  }, [changedParents]);

  if (isLoading || Object.keys(items).length === 0 || !dataProvider) {
    return <div>Loading...</div>;
  }

  const onItemSelected = (itemKeys: TreeItemIndex[]) => {
    // we don't understand how to multi-select at the moment, so we just take the first
    const selectedItem = items[itemKeys[0]];
    if (selectedItem.data.fileType !== FileType.Section) {
      setSelectedItem(selectedItem);
    }
  };

  return (
    <UncontrolledTreeEnvironment
      dataProvider={dataProvider}
      getItemTitle={(item) => item.data.name}
      viewState={{
        ["tree-1"]: {
          expandedItems: [],
        },
      }}
      onSelectItems={onItemSelected}
      {...renderers}
    >
      <ComplexTree treeId={treeId} rootItem={rootItem} treeLabel={treeLabel} />
    </UncontrolledTreeEnvironment>
  );
};

export default Tree;
