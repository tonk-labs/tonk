import React from "react";
import { Box, Text } from "ink";
import { Link } from "../components/Link";
import { ROUTE } from "../routes";
import { SelectionZone } from "../components/SelectionZone/SelectionZone";

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

        <Box marginY={2}>
          <SelectionZone nextKey="up" prevKey="down">
            <Box flexDirection="column" gap={1}>
              <Link to={ROUTE.CONVERT}>Convert Locations</Link>
            </Box>
          </SelectionZone>
        </Box>
      </Box>
      <Box alignSelf="flex-end">
        <Text>Ver. {version}</Text>
      </Box>
    </Box>
  );
};
