import React from "react";
import InkTextInput from "ink-text-input";

import { useDidMountEffect } from "../hooks";
import { isNumeric } from ".././utils/isNumeric";

interface InkTextInputProps {
  placeholder?: string;
  focus?: boolean;
  mask?: string;
  showCursor?: boolean;
  highlightPastedText?: boolean;
  value?: string;
  onChange: (value: string) => void;
  onSubmit?: (value: string) => void;
}

export interface InputProps extends InkTextInputProps {
  type?: "text" | "number";
  onFocus?: () => void;
  onBlur?: () => void;
  disabled?: boolean;
}

export const Input: React.FC<InputProps> = ({
  onFocus,
  onBlur,
  onChange,
  disabled,
  type = "text",
  ...props
}) => {
  const { focus } = props;

  useDidMountEffect(() => {
    if (focus) {
      onFocus && onFocus();
    } else {
      onBlur && onBlur();
    }
  }, [focus]);

  const handleChange = (value: string) => {
    if (disabled) return;

    if (type === "number") {
      value = value.replace(",", ".");

      if (value && !isNumeric(value)) {
        return;
      }
    }

    onChange(value);
  };

  return (
    <InkTextInput
      onChange={handleChange}
      {...props}
      value={props.value ?? ""}
    />
  );
};
