import React, { useEffect } from "react";
import { Text, TextProps } from "ink";

import { SuperKey, useKey } from "../hooks";

export interface TextButtonProps extends TextProps {
  children: React.ReactNode;
  onPress: () => void;
  isFocused?: boolean;
  selectKey?: SuperKey | SuperKey[];
  autoPress?: boolean;
}

export const TextButton: React.FC<TextButtonProps> = ({
  children,
  selectKey = "return",
  onPress,
  isFocused,
  autoPress,
  ...props
}) => {
  useKey(selectKey, onPress, isFocused);

  useEffect(() => {
    if (isFocused && autoPress) {
      onPress();
    }
  }, [autoPress, isFocused]);

  return (
    <Text {...props} bold={isFocused}>
      {children}
    </Text>
  );
};
