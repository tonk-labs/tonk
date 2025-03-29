import React, { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import styles from "./Welcome.module.css";
import { Text, RainbowMode, Button } from "../../components";
import { handleSelectDirectory } from "../../ipc/ui";
import { init, copyHubTemplate } from "../../ipc/hub";
import { useConfigStore } from "../../stores/configStore";

const Welcome: React.FC = () => {
  const [selectedPath, setSelectedPath] = useState("");
  const { isInitialized, isLoading, loadConfig } = useConfigStore();
  const navigate = useNavigate();

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

  return !isInitialized ? (
    <div className={styles.container}>
      <div className={styles.top}>
        <Text>Welcome to Tonk!</Text>
        <Text>
          We're excited to work with you in building your own&nbsp;
          <RainbowMode>☆Little Internet☆</RainbowMode>
        </Text>
      </div>

      <div className={styles.middle}>
        <Text>Please select a folder location for your new Tonk home.</Text>
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
  ) : null;
};

export default Welcome;
