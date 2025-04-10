import React from "react";
import { Boxes } from "cli-boxes";
import { Box, Text } from "ink";
import Spinner from "ink-spinner";

import { useInput } from "../hooks";

import { Error, Input, InputProps } from ".";

export interface InputBoxProps extends InputProps {
  label?: string;
  error?: string;
  loading?: boolean;
  postfix?: string;
  width?: string | number;
  borderStyle?: keyof Boxes;
  children?: React.ReactNode | React.ReactNode[];
}

export const InputBox: React.FC<InputBoxProps> = ({
  label,
  error,
  focus = false,
  loading = false,
  postfix = "",
  width,
  onChange,
  borderStyle = "classic",
  children,
  ...inputProps
}) => {
  const disabled = inputProps.disabled || loading;

  useInput(({ key }) => {
    if (key.ctrl && (key.backspace || key.delete)) {
      process.nextTick(() => onChange(""));
    }
  }, focus);

  return (
    <Box
      borderStyle={borderStyle}
      borderColor={disabled ? "grey" : undefined}
      flexDirection="column"
      width={width}
    >
      <Box>
        {label ? (
          <Box>
            <Text color={disabled ? "grey" : undefined}>{label}: </Text>
          </Box>
        ) : null}
        <Input
          focus={focus}
          disabled={disabled}
          onChange={onChange}
          {...inputProps}
        />
        {loading ? (
          <Box marginLeft={1}>
            <Spinner />
          </Box>
        ) : null}
        {!loading && postfix ? <Text>{postfix}</Text> : null}
      </Box>
      {children}
      {!loading && error ? <Error text={error} /> : null}
    </Box>
  );
};
