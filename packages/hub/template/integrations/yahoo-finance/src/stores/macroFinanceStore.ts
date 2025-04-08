import { create } from "zustand";
import { sync } from "@tonk/keepsync";
import { type TrendingSymbolsData } from "../workers/macroFinance";

type State = {
  data: TrendingSymbolsData | null;
  error: string | null;
  setData: (data: TrendingSymbolsData | null) => void;
  setError: (error: string | null) => void;
};

export const useMacroFinanceStore = create<State>(
  sync(
    (set) => ({
      data: null,
      error: null,
      setData: (data) => set({ data }),
      setError: (error) => set({ error }),
    }),
    {
      docId: "yahoo-finance-macro",
    }
  )
);
