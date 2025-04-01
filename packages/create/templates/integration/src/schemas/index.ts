import { z } from "zod";

const ExampleSchema = z.object({
  id: z.number(),
  name: z.string(),
  email: z.string().email(),
});

export default [
  {
    name: "ExampleSchema",
    schema: ExampleSchema,
  },
];
