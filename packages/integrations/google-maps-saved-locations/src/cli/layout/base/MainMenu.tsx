import React from "react";
import { Box, Text } from "ink";

import { Menu, MenuItem } from "../../components";
import { ROUTE, useNavigate } from "../../routes";
import { useSelectionZone } from "../../components/SelectionZone";
import { COLUMNS } from "../../constants";

export const MainMenu: React.FC = () => {
  const navigate = useNavigate();
  const parentZone = useSelectionZone()!;

  const menuItems: MenuItem[] = [
    {
      title: "Home",
      onSelect: () => {
        navigate(ROUTE.HOME);
        parentZone.select(COLUMNS.MAIN);
      },
    },
    {
      title: "Convert",
      onSelect: () => {
        navigate(ROUTE.CONVERT);
        parentZone.select(COLUMNS.MAIN);
      },
    },
    {
      title: "COLUMN",
      items: [],
    },
    {
      title: "Help",
      onSelect: () => {
        navigate(ROUTE.HELP);
        parentZone.select(COLUMNS.MAIN);
      },
    },
  ];

  return (
    <>
      <Box alignSelf="center" marginTop={-1}>
        <Text bold> Menu </Text>
      </Box>
      <Menu
        isActive={parentZone.selection === COLUMNS.MENU}
        items={menuItems}
        prevKey="upArrow"
        nextKey="downArrow"
        selectKey={["return", "rightArrow"]}
        expandMenuKey={["return", "rightArrow"]}
      />
    </>
  );
};
