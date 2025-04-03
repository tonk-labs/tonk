import React, { useEffect, useRef } from "react";

export const useDidMountEffect = (
  func: React.EffectCallback,
  deps?: React.DependencyList
) => {
  const didMount = useRef(false);

  useEffect(() => {
    if (didMount.current) {
      func();
    } else {
      didMount.current = true;
    }
  }, deps);
};
