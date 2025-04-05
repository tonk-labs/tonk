import { z } from "zod";

const TrendingSymbolSchema = z.object({
  symbol: z.string(),
});

const TrendingSymbolResponseSchema = z.object({
  count: z.number(),
  quotes: z.array(TrendingSymbolSchema),
  jobTimestamp: z.number(),
  startInterval: z.number(),
});

export default [
  {
    name: "TrendingSymbolSchema",
    schema: TrendingSymbolResponseSchema,
  },
];
