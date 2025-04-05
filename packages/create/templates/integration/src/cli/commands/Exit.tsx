import React, { useEffect } from "react";
import { useInkStore } from "../stores/inkStore";

export const Exit = () => {
  const { instance } = useInkStore();
  useEffect(() => {
    instance?.unmount();
  });
  return null;
};

export default Exit;
