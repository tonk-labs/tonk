import React, { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { ActionBar, ContentArea, Tree } from "../../components";
import { useConfigStore } from "../../stores/configStore";
import styles from "./Home.module.css";
import { runServer } from "../../ipc/hub";

const Home: React.FC = () => {
  const { isInitialized, isLoading, loadConfig } = useConfigStore();

  const navigate = useNavigate();
  useEffect(() => {
    loadConfig();
    runServer();
  }, []);

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
      </div>
      <ContentArea />
    </div>
  );
};

export default Home;
