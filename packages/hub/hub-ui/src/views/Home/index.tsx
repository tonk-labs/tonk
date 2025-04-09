import React, { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { ActionBar, Button, ContentArea, Tree } from "../../components";
import { useConfigStore } from "../../stores/configStore";
import styles from "./Home.module.css";
import { RefreshCcw } from "lucide-react";
import { runServer } from "../../ipc/hub";

const Home: React.FC = () => {
    const { isInitialized, isLoading, loadConfig, clearConfig } =
        useConfigStore();

    const navigate = useNavigate();
    useEffect(() => {
        loadConfig();
        runServer();
    }, []);

    const handleLogout = () => {
        if (
            confirm(
                "Are you sure you want to clear your data? This will delete all your projects."
            )
        ) {
            clearConfig();
            navigate("/");
        }
    };

    useEffect(() => {
        loadConfig();
    }, []);

    useEffect(() => {
        if (!isInitialized && !isLoading) {
            navigate("/");
        }
    }, [isInitialized, isLoading, navigate]);
    if (isLoading || !isInitialized) {
        return null;
    }
    return (
        <div className={styles.container}>
            <div className={styles.sidebar}>
                <ActionBar />
                <Tree />
                <div
                    style={{
                        height: "100%",
                        display: "flex",
                        alignItems: "flex-end",
                    }}
                >
                    <Button onClick={handleLogout} color="ghost" size="sm">
                        Clear data
                    </Button>
                </div>
            </div>
            <ContentArea />
        </div>
    );
};

export default Home;
