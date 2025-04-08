import React, { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { ActionBar, Button, ContentArea, Tree } from "../../components";
import { useConfigStore } from "../../stores/configStore";
import styles from "./Home.module.css";

const Home: React.FC = () => {
    const { isInitialized, isLoading, loadConfig, clearConfig } =
        useConfigStore();

    const navigate = useNavigate();

    const handleLogout = async () => {
        await clearConfig();
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
                <Button
                    variant="ghost"
                    size="sm"
                    shape="square"
                    title="Logout"
                    onClick={handleLogout}
                >
                    logout
                </Button>
            </div>
            <ContentArea />
        </div>
    );
};

export default Home;
