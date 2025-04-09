import React, { useEffect, useMemo } from "react";
import {
    Tree as ComplexTree,
    StaticTreeDataProvider,
    TreeItemIndex,
    UncontrolledTreeEnvironment,
} from "react-complex-tree";
import "react-complex-tree/lib/style-modern.css";
import { useProjectStore } from "../../stores/projectStore";
import { renderers } from "./renderers";
import Button from "../Button";
import { RefreshCcw } from "lucide-react";

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
    treeLabel = "Projects",
}) => {
    const {
        items: _items,
        loadProjects,
        isLoading,
        changedParents,
        setSelectedItem,
    } = useProjectStore();
    const items = useMemo(() => {
        return { ..._items };
    }, [_items]);
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

    if (isLoading) {
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
        <div>
            <div
                style={{
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    position: "relative",
                    padding: "6px 12px",
                    paddingBottom: "0px",
                }}
            >
                <h2>{treeLabel}</h2>
            </div>
            {Object.keys(items).length === 0 || !dataProvider ? (
                <div>No items to display.</div>
            ) : (
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
                    <ComplexTree
                        treeId={treeId}
                        rootItem={rootItem}
                        treeLabel={treeLabel}
                    />
                </UncontrolledTreeEnvironment>
            )}
        </div>
    );
};

export default Tree;
