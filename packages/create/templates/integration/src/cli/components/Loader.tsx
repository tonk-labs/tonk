import React from "react";
import type { SpinnerName } from "cli-spinners";
import Spinner from "ink-spinner";

interface Props {
  loading?: boolean;
  type?: SpinnerName;
  children: React.ReactNode | React.ReactNode[];
}

export const Loader: React.FC<Props> = ({ loading, type, children }) => {
  return loading ? <Spinner type={type} /> : <>{children}</>;
};
