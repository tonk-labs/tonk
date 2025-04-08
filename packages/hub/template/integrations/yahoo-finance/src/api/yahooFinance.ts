// import syntax (recommended)
import yahooFinance from "yahoo-finance2";

export const getTrendingSymbols = async () => {
  const queryOptions = { count: 5, lang: "en-US" };
  return await yahooFinance.trendingSymbols("US", queryOptions);
};
