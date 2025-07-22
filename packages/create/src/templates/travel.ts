import { ProjectPlan } from "../types";
import { createTemplate, TemplateConfig } from "./base";

export async function createTravelTemplate(
  projectPath: string,
  projectName: string,
  templatePath: string,
  plan: ProjectPlan,
) {
  const config: TemplateConfig = {
    type: "travel-planner",
    displayName: "Travel Planner",
    successMessage: "🎉 Your travel planner is ready for vibe coding! 🎉",
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
