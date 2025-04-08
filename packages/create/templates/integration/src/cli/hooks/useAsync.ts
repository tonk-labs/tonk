import { useState } from "react";

export const useAsync = <Args extends any[], Return>(
  func: (...args: Args) => Promise<Return>
) => {
  const [state, setState] = useState({
    isLoading: false,
    error: "",
  });

  const execute = async (...args: Args): Promise<Return> => {
    setState({ isLoading: true, error: "" });

    try {
      const result = await func(...args);
      setState({ isLoading: false, error: "" });

      return Promise.resolve(result);
    } catch (err: any) {
      setState({
        isLoading: false,
        error: err?.message ?? err?.toString() ?? "Unknown error",
      });

      return Promise.reject(err);
    }
  };

  const safeExecute = (...args: Args) => {
    return execute(...args).catch(() => null);
  };

  const clearError = () => {
    setState({ ...state, error: "" });
  };

  return {
    execute,
    safeExecute,
    clearError,
    ...state,
  };
};
