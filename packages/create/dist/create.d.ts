interface ProjectPlan {
    implementationLog: string[];
    projectDescription: string;
}
type TemplateType = "default" | "React-PWA";

declare function createProject(projectName: string, plan: ProjectPlan, _templateName: TemplateType): Promise<void>;

export { createProject };
