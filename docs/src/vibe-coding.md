# Vibe Coding with Tonk

## What is Vibe Coding?

Vibe coding is a novel approach to software development where a person uses natural language to describe tasks to an LLM which generates the code. The person is primarily responsible for guiding the LLM and imparting a vision. Vibe coding is about building momentum, following creative instincts, and evolving a project through rapid iteration and feedback loops.

## Essential Tools

### IDEs
- [**Cursor**](https://www.cursor.com/en) - fork of Visual Studio Code, with deep AI integration
- [**Windsurf**](https://windsurf.com/) - another VS Code fork, more streamlined and beginner-friendly

### CLI
- [**Claude Code**](https://github.com/anthropics/claude-code) - highly capable and intuitive CLI-based agentic coding tool

### Models
- **Claude 3.7 Sonnet** or **Claude 4 Sonnet** recommended for most tasks
- **Claude 4 Opus** or **OpenAI o3** for complicated reasoning tasks and planning

## Core Principles

### 1. Begin with a plan

Using your favourite vibe coding IDE or chat interface, work out a detailed plan of what you want to achieve with your project. It's best to include features, views, technical details, constraints, and visual preferences. If you're unsure about any specifics, have the LLM go with its best judgement. You can always refine your project later.

Once you're happy with your plan, save it to a markdown file in your project, so the LLM can reference it later. Then, open your agentic editor to your Tonk project and prompt the LLM with the first part of your plan.

### 2. Retain control

Vibe coding at its best can fully abstract the code from you. It should never abstract intent. LLMs can only implement what you can articulate, so make sure you can reason about what you're asking of it. Vibe coding fails hard and fast when you lose understanding of what the LLM is doing at a high level.

Break tasks into small steps you know how to test. When something goes wrong, specifically articulate what you wanted, what you observed, and what you want to change. If there are errors in the browser, code, or terminal, make sure to pass them on to the LLM.

### 3. Git as fail-safe

Each time you complete a milestone, commit the changes to git. This allows you to quickly see what changes the LLM has made since the last commit, compare various stages of your project, and try multiple iterations of a feature without risking stable parts of your code.

### 4. Ask AI for help

If you're not sure what you want or how to get there, be as clear as you can to the LLM and ask for some options without writing any code. Weigh the options then ask it to go through with one. If you want to try multiple, commit your changes to git first. Then, you can change the code as much as you like and revert when necessary.

You can also ask the LLM to explain any part of the code to you. If you feel yourself losing grip of the project's intent, don't be afraid to dig into the code and poke around.

## Where Tonk Comes In

Tonk is designed specifically to enable vibe coding workflows. Here's how Tonk supports the vibe coding approach:

### Streamlined Tech Stack

Tonk's architecture, tooling, and developer experience are tailored for easy use by LLMs. The entire backend is abstracted, letting the AI focus on what it does best: React frontends. We use Vite, Tailwind, and Zustand, which are favoured by agentic tooling and provide the optimal balance between convenience and extensibility.

Tonk provides `sync` middleware for Zustand stores so that all state saved to a store is automatically persisted and synced between connected devices around the world.

A backend server is automatically generated so you can query external services from your React app.

### Exploratory Development
- **Rapid prototyping** - spin up new ideas in minutes, not hours
- **Easy experimentation** - try different approaches without fear of breaking things
- **Seamless scaling** - move from prototype to production without architectural rewrites

## What's Achievable with Tonk

### Perfect For
- **Real-time applications** - chat apps, collaborative tools, live dashboards
- **Exploratory projects** - when you're not sure what you're building yet
- **Idiosyncratic expression** - for a unique and creative web
- **Learning and experimentation** - try new ideas quickly
- **Small to medium applications** - full-stack apps with 1-1000 users
- **Hackathons and time-boxed projects** - maximum velocity development

### Coming Soon
- **Enterprise applications** - greater security guarantees and reliability
- **User space** - tying users to data
- **Identity, authentication, permissions** - gate access and share with peace of mind

### Don't Use For
- **Mission-critical systems**
- **Performance-critical applications**
- **Massive scale**

## Resources
For the practical: https://github.com/techiediaries/awesome-vibe-coding?tab=readme-ov-file.

For the poetic: https://www.thewayofcode.com/
