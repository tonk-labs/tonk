export interface ProjectPlan {
  implementationLog: string[];
  projectDescription: string;
}

export type TemplateType = "default" | "React-PWA" | "react" | "integration";

export interface TemplateConfig {
  name: string;
  plan: ProjectPlan;
  template: TemplateType;
}
