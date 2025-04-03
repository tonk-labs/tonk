import React from "react";
import { Text, TextProps } from "ink";

import { SuperKey, useKey } from "../hooks";
import { ROUTE, useNavigate } from "../routes";
import { Selection } from "./SelectionZone/Selection";

interface Props extends TextProps {
  children: React.ReactNode;
  to: ROUTE;
  selectKey?: SuperKey | SuperKey[];
  isFocused?: boolean;
}

export const Link: React.FC<Props> = ({
  children,
  selectKey = "return",
  to,
  isFocused: propIsFocused,
  ...props
}) => {
  const navigate = useNavigate();

  const handlePress = () => {
    navigate(to);
  };

  return (
    <Selection>
      {(isFocused) => {
        useKey(selectKey, handlePress, isFocused);

        return (
          <Text
            {...props}
            bold={propIsFocused !== undefined ? propIsFocused : isFocused}
          >
            {children}
          </Text>
        );
      }}
    </Selection>
  );
};
