import React from "react";
import { Box, Text } from "ink";

export const Help: React.FC = () => {
  return (
    <Box display="flex" flexDirection="column">
      <Box marginTop={-1}>
        <Text bold> Help </Text>
      </Box>
      <Box display="flex" flexDirection="column" paddingX={1}>
        <Text>
          Use <Text bold>&quot;Enter&quot;</Text> to perform action
        </Text>
        <Text>
          Use <Text bold>&quot;Tab&quot;</Text> to move between menus
        </Text>
        <Text>
          Use <Text bold>&quot;↑&quot;</Text> / <Text bold>&quot;↓&quot;</Text>{" "}
          / <Text bold>&quot;←&quot;</Text> / <Text bold>&quot;→&quot;</Text> to
          move to select inputs or buttons
        </Text>
        <Text>
          Use <Text bold>&quot;Ctrl + Backspace&quot;</Text> to clear input
        </Text>
      </Box>
    </Box>
  );
};
