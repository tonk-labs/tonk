import React, { useEffect } from "react";
import {
  UncontrolledTreeEnvironment,
  Tree as ComplexTree,
  StaticTreeDataProvider,
  TreeItemIndex,
} from "react-complex-tree";
import "react-complex-tree/lib/style-modern.css";
import { renderers } from "./renderers";
import { useProjectStore } from "../../stores/projectStore";

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
  const { items, loadProjects, isLoading, setSelectedItem } = useProjectStore();

  useEffect(() => {
    loadProjects();
  }, []);

  if (isLoading || Object.keys(items).length === 0) {
    return <div>Loading...</div>;
  }

  const dataProvider = new StaticTreeDataProvider(items, (item, newName) => ({
    ...item,
    data: {
      ...item.data,
      name: newName,
    },
  }));

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
      viewState={{}}
      onSelectItems={onItemSelected}
      {...renderers}
    >
      <ComplexTree treeId={treeId} rootItem={rootItem} treeLabel={treeLabel} />
    </UncontrolledTreeEnvironment>
  );
};

export default Tree;
