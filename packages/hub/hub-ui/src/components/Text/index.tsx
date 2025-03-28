import React from "react";
import styles from "./Text.module.css";

interface TextProps {
  children: string | React.ReactNode;
  rainbowMode?: boolean;
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

export const RainbowMode: React.FC<TextProps> = ({ children }) => {
  if (!children) return "";

  if (typeof children !== "string") return "";

  return [
    children.split("").map((char, index) => (
      <span
        key={index}
        className={styles.rainbowChar}
        style={
          {
            color: RAINBOW_COLORS[index % RAINBOW_COLORS.length],
            "--char-index": index,
          } as React.CSSProperties
        }
      >
        {char}
      </span>
    )),
  ];
};

export const Text: React.FC<TextProps> = ({ children }) => (
  <p className={styles.text}>{children}</p>
);

export default Text;
