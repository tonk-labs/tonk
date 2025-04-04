import React, { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import styles from "./Home.module.css";
import { Tree, ContentArea, ActionBar } from "../../components";
import { useConfigStore } from "../../stores/configStore";

const Home: React.FC = () => {
  const { isInitialized, isLoading, loadConfig } = useConfigStore();
  const navigate = useNavigate();

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
        <Tree />
        <ActionBar />
      </div>
      <ContentArea />
    </div>
  );
};

export default Home;
