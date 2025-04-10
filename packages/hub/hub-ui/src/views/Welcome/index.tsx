import React, { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Button, RainbowMode, Text, TypewriterText } from "../../components";
import { copyHubTemplate, init } from "../../ipc/hub";
import { handleSelectDirectory } from "../../ipc/ui";
import { useConfigStore } from "../../stores/configStore";
import styles from "./Welcome.module.css";

const Welcome: React.FC = () => {
    const [selectedPath, setSelectedPath] = useState("");
    const [step, setStep] = useState(0);
    const { isInitialized, isLoading, loadConfig } = useConfigStore();
    const [isCopying, setIsCopying] = useState(false);
    const navigate = useNavigate();
    const advanceStep = (newStep: number) => {
        if (step >= 4) {
            return;
        }
        if (step > newStep) {
            return;
        }
        // add a 100ms delay
        setTimeout(() => {
            setStep(newStep);
        }, 250);
    };

    useEffect(() => {
        if (step == 2) {
            new Promise((resolve) => {
                setTimeout(() => {
                    resolve(true);
                }, 1000);
            }).then(() => {
                setStep(3);
            });
        }
    }, [step]);

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

    const initialize = async () => {
        setIsCopying(true);
        await init(selectedPath);
        await copyHubTemplate();
        setIsCopying(false);
        navigate("/home");
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
                <TypewriterText delay={70} onComplete={() => advanceStep(1)}>
                    Welcome to Tonk!
                </TypewriterText>
                {step >= 1 && (
                    <div>
                        <TypewriterText
                            delay={35}
                            onComplete={() => advanceStep(2)}
                        >
                            {
                                "We're excited to work with you in building your own: "
                            }
                        </TypewriterText>
                    </div>
                )}
                {step >= 2 && (
                    <div
                        style={{
                            display: "flex",
                            flexDirection: "row",
                            justifyContent: "center",
                            alignItems: "center",
                            width: "100%",
                        }}
                    >
                        <RainbowMode style={{ fontSize: 24 }}>
                            ☆Little Internet☆
                        </RainbowMode>
                    </div>
                )}
            </div>

            <>
                <div className={styles.middle}>
                    <Text>
                        {step >= 3 && (
                            <TypewriterText
                                delay={35}
                                onComplete={() => advanceStep(4)}
                                style={{ color: "gray", fontSize: "12px" }}
                            >
                                Choose where to store your Tonk data:
                            </TypewriterText>
                        )}
                        {step >= 4 && (
                            <div
                                style={{
                                    display: "flex",
                                    flexDirection: "row",
                                    alignItems: "center",
                                    marginTop: "0.25em",
                                    marginLeft: "0.25em",
                                }}
                            >
                                <Button size="md" onClick={handleSelectFolder}>
                                    <FolderButton
                                        size={32}
                                        onClick={handleSelectFolder}
                                    />

                                    {selectedPath !== ""
                                        ? `${selectedPath}/tonk`
                                        : ""}
                                </Button>
                            </div>
                        )}
                    </Text>
                </div>
                {step >= 4 && (
                    <div className={styles.bottom}>
                        <Button
                            color="green"
                            size="lg"
                            shape="default"
                            onClick={initialize}
                            disabled={isCopying}
                        >
                            {isCopying ? "Setting up your hub..." : "Continue"}
                        </Button>
                    </div>
                )}
            </>
        </div>
    );
};

export default Welcome;

const FolderButton: React.FC<{ size: number; onClick: () => void }> = ({
    size,
    onClick,
}) => {
    return (
        <svg
            width={size}
            height={size}
            viewBox="0 0 24 24"
            style={{
                marginRight: "8px",
                verticalAlign: "middle",
                fill: "rgba(255, 255, 255, 0.7)",
                opacity: 0.8,
                transition: "all 0.2s ease",
            }}
            onMouseEnter={(e) => {
                e.currentTarget.style.fill = "rgba(255, 255, 255, 0.9)";
                e.currentTarget.style.opacity = "1";
            }}
            onMouseLeave={(e) => {
                e.currentTarget.style.fill = "rgba(255, 255, 255, 0.7)";
                e.currentTarget.style.opacity = "0.8";
            }}
        >
            <path d="M20 6h-8l-2-2H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V6h5.17l2 2H20v10z" />
        </svg>
    );
};
