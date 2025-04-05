import React, { useState } from "react";
import { Box, Text } from "ink";
import SelectInput from "ink-select-input";
import commands from "./commands";

type SelectItem = {
  label: string;
  value: string;
};

const renderPossibleCommands = (handleSelect: (item: SelectItem) => void) => {
  const items = commands.map((cmd) => ({
    label: cmd.label,
    value: cmd.label,
  }));
  return (
    <Box flexDirection="column" marginBottom={1}>
      <Text>Please select the command to run...</Text>
      <SelectInput items={items} onSelect={handleSelect} />
    </Box>
  );
};

export const App: React.FC = () => {
  const [route, setRoute] = useState("cli");

  if (route === "cli") {
    return renderPossibleCommands((item: SelectItem) => {
      setRoute(item.value);
    });
  } else {
    const cmd = commands.find((cmd) => cmd.label === route);
    if (!cmd) return null;
    const Component = cmd.value;
    return <Component />;
  }
};

export default App;
