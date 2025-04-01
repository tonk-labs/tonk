import React from "react";
import { Box, Text } from "ink";

export const Header: React.FC = () => {
  return (
    <Box flexDirection="column" borderStyle="single" paddingX={1}>
      <Box marginTop={-1} justifyContent="space-between">
        <Text bold> {"Header Title"} </Text>
      </Box>
      <Box flexDirection="row" justifyContent="space-between">
        <Text color="cyan">{"Some Information"}</Text>
      </Box>
    </Box>
  );
};
