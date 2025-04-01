import React from "react";
import { Box, Text } from "ink";
import AsciiText from "../components/AsciiText";

import { GITHUB } from "../constants";

import { version } from "../../../package.json";

export const Home: React.FC = () => {
  return (
    <Box
      display="flex"
      flexDirection="column"
      justifyContent="space-between"
      height="95%"
    >
      <Box flexDirection="column">
        <Text>
          <Text bold>Github:</Text> <Text underline>{GITHUB}</Text>
        </Text>
      </Box>
      <Box alignSelf="flex-end">
        <Text>Ver. {version}</Text>
      </Box>
    </Box>
  );
};
