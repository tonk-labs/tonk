import React from "react";
import { Box, Text } from "ink";
import { TextButton } from "../components";
import AsciiText from "../components/AsciiText";

import { ROUTE, useNavigate } from "../routes";
// import { TextButton } from "@src/components/TextButton";
// import { restoreAppState, useAppStore } from "@src/store";

import { version } from "../../../package.json";

export const Welcome: React.FC = () => {
  const navigate = useNavigate();

  const onEnter = () => {
    navigate(ROUTE.HOME);
  };

  return (
    <Box flexDirection="column" justifyContent="center" alignItems="center">
      <Box flexDirection="column" marginBottom={1}>
        <AsciiText text="Tonk Integrations" />
      </Box>
      <TextButton selectKey="any" onPress={onEnter} isFocused>
        Press any key to continue...
      </TextButton>
      <Box marginTop={2}>
        <Text>Ver. {version}</Text>
      </Box>
    </Box>
  );
};
