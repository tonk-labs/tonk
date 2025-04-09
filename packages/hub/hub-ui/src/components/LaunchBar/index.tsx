import React from "react";
import styles from "./LaunchBar.module.css";
import { Button } from "../";
import { TreeItem, FileType } from "../Tree";
import { launchApp } from "../../ipc/app";
import { getConfig } from "../../ipc/config";
import { platformSensitiveJoin } from "../../ipc/files";
import { useProjectStore } from "../../stores/projectStore";

const AppLaunchBar = (props: {
    selectedItem: TreeItem;
    commandCallback: (cmd: string) => void;
}) => {
    const { selectedItem, commandCallback } = props;
    const launch = async () => {
        const config = await getConfig();
        const fullPath = await platformSensitiveJoin([
            config!.homePath,
            selectedItem.index,
        ]);
        await launchApp(fullPath!);
    };

    return (
        <div className={styles.buttonArea}>
            <Button color="green" size="sm" shape="rounded" onClick={launch}>
                Launch
            </Button>
        </div>
    );
};

const LaunchBar = (props: { commandCallback: (cmd: string) => void }) => {
    const { selectedItem } = useProjectStore();
    const { commandCallback } = props;
    if (!selectedItem) {
        return null;
    }
    switch (selectedItem.data.fileType) {
        case FileType.App: {
            return (
                <AppLaunchBar
                    selectedItem={selectedItem}
                    commandCallback={commandCallback}
                />
            );
        }
        default: {
            return null;
        }
    }
};

export default LaunchBar;
