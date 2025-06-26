import { ProjectPlan } from "../types";
import { createTemplate, TemplateConfig } from "./base";

export async function createReactTemplate(
  projectPath: string,
  projectName: string,
  templatePath: string,
  plan: ProjectPlan,
) {
  const config: TemplateConfig = {
    type: "react",
    displayName: "React",
    successMessage: "🎉 Your Tonk react app is ready for vibe coding! 🎉",
    nextSteps: [
      {
        command: "pnpm dev",
        description: "Start the development server",
      },
      {
        command: "pnpm build",
        description: "Build your project for production",
      },
    ],
  };

  await createTemplate(projectPath, projectName, templatePath, plan, config);
}
