import React, { useEffect, useRef, useState } from "react";
import { MaybePromise } from "tsdef";

import { Button, ButtonProps } from ".";

export interface AsyncButtonProps extends Omit<ButtonProps, "onPress"> {
  onPress: () => MaybePromise<unknown>;
}

const AsyncButton: React.FC<AsyncButtonProps> = ({
  children,
  onPress,
  ...props
}) => {
  const [disabledByPromise, setDisabledByPromise] = useState(false);
  const mounted = useRef(false);

  const asyncOnPress = async () => {
    setDisabledByPromise(true);
    try {
      await onPress();
    } finally {
      mounted.current && setDisabledByPromise(false);
    }
  };

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  });

  return (
    <Button
      {...props}
      isLoading={disabledByPromise || props.isLoading}
      onPress={asyncOnPress}
    >
      {children}
    </Button>
  );
};

export default AsyncButton;
