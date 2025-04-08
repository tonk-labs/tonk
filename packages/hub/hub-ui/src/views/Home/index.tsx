import React, { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import styles from "./Home.module.css";
import { Tree, ContentArea, ActionBar, Button } from "../../components";
import { useConfigStore } from "../../stores/configStore";
import { HomeIcon } from "lucide-react";
import { useProjectStore } from "../../stores/projectStore";

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
        <ActionBar />
        <Tree />
      </div>
      <ContentArea />
    </div>
  );
};

export default Home;
