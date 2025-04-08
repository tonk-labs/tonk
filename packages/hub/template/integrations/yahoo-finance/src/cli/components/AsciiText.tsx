import React, { useEffect, useState } from "react";
import { Text } from "ink";
import figlet from "figlet";

export interface AsciiTextProps {
  text: string;
  font?: figlet.Fonts;
  align?: "left" | "center" | "right";
  color?: string;
}

export const AsciiText: React.FC<AsciiTextProps> = ({
  text,
  font = "Standard",
  align = "left",
  color,
}) => {
  const [asciiArt, setAsciiArt] = useState<string>("");

  useEffect(() => {
    figlet.text(text, { font }, (err: Error | null, result?: string) => {
      if (err) {
        console.error("Something went wrong with figlet", err);
        return;
      }
      setAsciiArt(result || "");
    });
  }, [text, font]);

  return <Text color={color}>{asciiArt}</Text>;
};

export default AsciiText;
