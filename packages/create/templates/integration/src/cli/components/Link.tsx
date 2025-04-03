import React from "react";
import { Text, TextProps } from "ink";

import { SuperKey, useKey } from "../hooks";
import { ROUTE, useNavigate } from "../routes";

interface Props extends TextProps {
  children: React.ReactNode;
  isFocused: boolean;
  to: ROUTE;
  selectKey?: SuperKey | SuperKey[];
}

export const Link: React.FC<Props> = ({
  children,
  selectKey = "return",
  isFocused,
  to,
  ...props
}) => {
  const navigate = useNavigate();

  useKey(selectKey, () => navigate(to), isFocused);

  return (
    <Text {...props} bold={isFocused}>
      {children}
    </Text>
  );
};
