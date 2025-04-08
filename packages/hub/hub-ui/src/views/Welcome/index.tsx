import React, { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Button, RainbowMode, Text } from "../../components";
import { copyHubTemplate, init } from "../../ipc/hub";
import { handleSelectDirectory } from "../../ipc/ui";
import { useConfigStore } from "../../stores/configStore";
import styles from "./Welcome.module.css";

const Welcome: React.FC = () => {
    const [selectedPath, setSelectedPath] = useState("");
    const { isInitialized, isLoading, loadConfig } = useConfigStore();
    const navigate = useNavigate();

    useEffect(() => {
        // Set default path to Documents folder
        try {
            const documentsPath = window.electronAPI.getDocumentsPath();
            setSelectedPath(documentsPath);
        } catch (error) {
            console.error("Error getting documents path:", error);
        }
    }, []);

    const handleSelectFolder = () => {
        handleSelectDirectory((path: string) => {
            setSelectedPath(path);
        });
    };

    const initialize = () => {
        init(selectedPath).then(() => {
            copyHubTemplate().then(() => {
                navigate("/home");
            });
        });
    };

    useEffect(() => {
        loadConfig();
    }, []);

    useEffect(() => {
        if (isInitialized) {
            navigate("/home");
        }
    }, [isInitialized, navigate]);

    if (isLoading) {
        return null;
    }

    if (isInitialized) {
        return null;
    }

    return (
        <div className={styles.container}>
            <div className={styles.top}>
                <Text>Welcome to Tonk!</Text>
                <Text>
                    We're excited to work with you in building your own&nbsp;
                    <RainbowMode>☆Little Internet☆</RainbowMode>
                </Text>
            </div>

            <div className={styles.middle}>
                <Text>
                    Please select a folder location for your new Tonk home.
                </Text>
                <Button variant="purple" onClick={handleSelectFolder}>
                    Browse
                </Button>
                <Text>
                    <span style={{ fontWeight: 700 }}>
                        Your home will be created at:{" "}
                    </span>
                    {selectedPath !== "" ? `${selectedPath}/.tonk` : ""}
                </Text>
            </div>

            <div className={styles.bottom}>
                <Button variant="green" onClick={initialize}>
                    Create
                </Button>
            </div>
        </div>
    );
};

export default Welcome;
