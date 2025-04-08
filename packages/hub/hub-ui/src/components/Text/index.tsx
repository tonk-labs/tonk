import React from "react";
import styles from "./Text.module.css";

interface TextProps extends React.HTMLAttributes<HTMLParagraphElement> {
    children: string | React.ReactNode;
    rainbowMode?: boolean;
    fontSize?: number;
}

const RAINBOW_COLORS = [
    "#FF0000", // Red
    "#FF7F00", // Orange
    "#FFFF00", // Yellow
    "#35FF3C", // Green
    "#00C8FF", // Blue
    "#5a5aFF", //Indigo
    "#CF66FF", // Violet
];

export const RainbowMode: React.FC<TextProps> = ({ children, ...props }) => {
    if (!children) return "";

    if (typeof children !== "string") return "";

    return [
        children.split("").map((char, index) => (
            <span
                key={index}
                className={styles.rainbowChar}
                style={
                    {
                        ...props,
                        color: RAINBOW_COLORS[index % RAINBOW_COLORS.length],
                        "--char-index": index,
                        fontSize: "36pt",
                    } as React.CSSProperties
                }
            >
                {char === " " ? "\u00A0" : char}
            </span>
        )),
    ];
};

export const Text: React.FC<TextProps> = ({ children }) => (
    <div className={styles.text}>{children}</div>
);

export default Text;
