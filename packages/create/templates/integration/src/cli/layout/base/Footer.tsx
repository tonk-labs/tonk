import React from "react";
import { Box, Text } from "ink";

import { useKey } from "../../hooks";
import { ROUTE, useNavigate } from "../../routes";

export const Footer: React.FC = () => {
  const navigate = useNavigate();

  const handleHelpOpen = () => {
    navigate(ROUTE.HELP);
  };

  useKey("ctrl+h", handleHelpOpen);

  return (
    <Box flexDirection="column" borderStyle="single" paddingX={1}>
      <Box flexDirection="row" justifyContent="space-between">
        <Text>
          <Text bold>&quot;Tab&quot;</Text> to switch
        </Text>
        <Text>
          <Text bold>&quot;↑&quot;</Text> / <Text bold>&quot;↓&quot;</Text> to
          move
        </Text>
      </Box>
    </Box>
  );
};
