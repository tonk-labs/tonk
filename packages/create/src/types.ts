export interface ProjectPlan {
  implementationLog?: string[];
  projectDescription?: string;
  description?: string;
  platform?: string;
  projectName?: string;
  workerDependencies?: string[];
}

export type TemplateType = "react" | "worker" | "workspace";

export interface TemplateConfig {
  name: string;
  plan: ProjectPlan;
  template: TemplateType;
}
