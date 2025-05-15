export interface ProjectPlan {
  implementationLog: string[];
  projectDescription: string;
}

export type TemplateType = "react" | "node" | "worker";

export interface TemplateConfig {
  name: string;
  plan: ProjectPlan;
  template: TemplateType;
}
