export interface ProjectPlan {
  implementationLog: string[];
  projectDescription: string;
}

export type TemplateType = "default" | "React-PWA" | "react";

export interface TemplateConfig {
  name: string;
  plan: ProjectPlan;
  template: TemplateType;
}
