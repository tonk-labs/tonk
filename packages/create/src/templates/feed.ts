import { ProjectPlan } from "../types";
import { createTemplate, TemplateConfig } from "./base";

export async function createFeedTemplate(
  projectPath: string,
  projectName: string,
  templatePath: string,
  plan: ProjectPlan,
) {
  const config: TemplateConfig = {
    type: "social-feed",
    displayName: "Social Feed",
    successMessage: "🎉 Your social feed is ready for vibe coding! 🎉",
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
