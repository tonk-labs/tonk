export interface ProjectPlan {
  implementationLog: string[];
  projectDescription: string;
}

export type TemplateType = "react" | "node";

export interface TemplateConfig {
  name: string;
  plan: ProjectPlan;
  template: TemplateType;
}
