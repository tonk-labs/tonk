import { Box, Text } from "ink";
import React, { useEffect, useState } from "react";

export const Basic = () => {
  const [frame, setFrame] = useState(0);
  const colors = ["green", "yellow", "blue", "magenta", "cyan"];
  const currentColor = colors[frame % colors.length];

  useEffect(() => {
    const timer = setInterval(() => {
      setFrame((f) => (f + 1) % colors.length);
    }, 500);

    return () => clearInterval(timer);
  }, []);

  return (
    <Box flexDirection="column" alignItems="center" padding={1}>
      <Text bold color={currentColor}>
        ✨ Hello, Terminal World! ✨
      </Text>
      <Box marginTop={1}>
        <Text dimColor>(Watch me change colors...)</Text>
      </Box>
    </Box>
  );
};

export default Basic;
