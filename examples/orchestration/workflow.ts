type Context = {
  template: string;
  prompt: string;
};

type Job = {
  jobId: number;
  dependencies: number[];
  context: Context;
};

type Workflow = Job[];
