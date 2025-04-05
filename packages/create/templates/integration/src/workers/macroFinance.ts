import { z } from "zod";
import schemas from "../schemas";
import { getTrendingSymbols } from "../api/yahoofinance";
import { useMacroFinanceStore } from "../stores/MacroFinanceStore";

const TrendingSymbolSchema = schemas[0];

export type TrendingSymbolsData = z.infer<typeof TrendingSymbolSchema.schema>;

export async function fetchTrendingSymbols(): Promise<void> {
  const { setError, setData } = useMacroFinanceStore.getState();

  try {
    setError(null);

    const response = await getTrendingSymbols();

    // Validate the response data against our schema (this is a useful extra check)
    const validatedData = TrendingSymbolSchema.schema.parse(response);
    setData(validatedData);
  } catch (error) {
    setError(
      error instanceof Error
        ? error.message
        : "Failed to fetch trending symbols"
    );
  }
}

/**
 *  This is the run function for this worker
 */
export default async () => {
  await fetchTrendingSymbols();
};
