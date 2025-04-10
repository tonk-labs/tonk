import React from "react";
import { Text, TextProps } from "ink";

interface Props extends TextProps {
  text: string;
}

export const Error: React.FC<Props> = ({ text, ...props }) => {
  return (
    <Text {...props} color="red">
      {text}
    </Text>
  );
};
