#!/usr/bin/env node
import React, { useState } from "react";
import { render, Box, Text } from "ink";
import { App } from "./app";
import { AsciiText, TextButton } from "./components";
import { version } from "../../package.json";
import { inkStore } from "./stores/inkStore";

function Welcome() {
  const [enter, hitEnter] = useState(false);
  const onEnter = () => {
    hitEnter(true);
  };
  return enter ? (
    <App />
  ) : (
    <Box flexDirection="column" justifyContent="center" alignItems="center">
      <Box flexDirection="column" marginBottom={1}>
        <AsciiText text="Starter Template" />
      </Box>
      <TextButton selectKey="any" onPress={onEnter} isFocused>
        Press any key to continue...
      </TextButton>
      <Box marginTop={2}>
        <Text>Ver. {version}</Text>
      </Box>
    </Box>
  );
}

const cli = async () => {
  const app = render(<Welcome />);
  inkStore.getState().setInstance(app);
  await app.waitUntilExit();
};

export default cli;
