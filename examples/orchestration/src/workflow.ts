enum Sources {
  KEEPSYNC,
}

type Template = {
  path: string;
  source: Sources;
};

type Context = {
  template: Template;
  prompt: string;
  dependencyContext?: boolean;
};

type Job = {
  jobId: number;
  dependencies?: number[];
  context: Context;
};

type Workflow = Job[];

export const workflow: Workflow = [
  {
    jobId: 0,
    context: {
      template: {
        path: "./templates/browser.md",
        source: Sources.KEEPSYNC,
      },
      prompt:
        "Search the web to find flights, hotels, and car rental for a one week London to Paris trip in mid-August",
    },
  },
  {
    jobId: 1,
    dependencies: [0],
    context: {
      template: {
        path: "./templates/emailer.md",
        source: Sources.KEEPSYNC,
      },
      prompt:
        "Find emails related to the travel bookings and forward them to my friends going on the trip",
      dependencyContext: true,
    },
  },
  {
    jobId: 2,
    dependencies: [1],
    context: {
      template: {
        path: "./templates/summary.md",
        source: Sources.KEEPSYNC,
      },
      prompt:
        "Write the full travel itinerary and the details of the bookings to a note I care share out with my friends",
      dependencyContext: true,
    },
  },
];
