import { RefreshCw } from "lucide-react";
import React, { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ActionBar, Button, ContentArea, Tree } from "../../components";
import { runServer } from "../../ipc/hub";
import { useConfigStore } from "../../stores/configStore";
import styles from "./Home.module.css";

const Home: React.FC = () => {
  const { isInitialized, isLoading, loadConfig, clearConfig } =
    useConfigStore();

  const [clearing, setClearing] = useState(false);
  const navigate = useNavigate();
  useEffect(() => {
    loadConfig();
    runServer();
  }, []);

  const handleLogout = async () => {
    if (
      confirm(
        "Are you sure you want to clear your data? This will delete all your projects."
      )
    ) {
      setClearing(true);
      await clearConfig();
      setClearing(false);
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
      {clearing && (
        <div className={styles.overlay}>
          <div className={styles.overlaySpinner}>
            <RefreshCw size={48} />
          </div>
          <div className={styles.overlayContent}>
            <h2>Clearing Data</h2>
          </div>
        </div>
      )}
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
