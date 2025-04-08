import { z } from "zod";
import schemas from "../schemas";
import { getTrendingSymbols } from "../api/yahoofinance";
import { useMacroFinanceStore } from "../stores/MacroFinanceStore";

const TrendingSymbolSchema = schemas[0];

export type TrendingSymbolsData = z.infer<typeof TrendingSymbolSchema.schema>;
let counter = 0;

export async function fetchTrendingSymbols(): Promise<void> {
  const { setError, setData, data } = useMacroFinanceStore.getState();
  console.log(data);

  try {
    setError(null);

    // const response = await getTrendingSymbols();
    const response = {
      count: 1,
      quotes: [
        {
          symbol: `SUP:${counter}`,
        },
      ],
      jobTimestamp: 0,
      startInterval: 0,
    };

    // Validate the response data against our schema (this is a useful extra check)
    const validatedData = TrendingSymbolSchema.schema.parse(response);
    console.log("fetching trendSymbols", validatedData);
    setData(validatedData);
  } catch (error) {
    console.error(error);
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
