```
   ・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜

          _____   U  ___ u  _   _       _  __
         |_ " _|   \/"_ \/ | \ |"|     |"|/ /
           | |     | | | |<|  \| |>    | ' /
          /| |\.-,_| |_| |U| |\  |u  U/| . \\u
         u |_|U \_)-\___/  |_| \_|     |_|\_\
         _// \\_     \\    ||   \\,-.,-,>> \\,-.
        (__) (__)   (__)   (_")  (_/  \.)   (_/

   ・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜
```

Build your own ☆𝐿𝒾𝓉𝓉𝓁𝑒 𝐼𝓃𝓉𝑒𝓇𝓃𝑒𝓉☆ with **Tonk**

Today, that means an AI code writer friendly react-framework powered by a flexible sync-engine system.

The goal of **Tonk** is to build a **maximally interoperable network** where you unlock and capture the full value of your data; addressing the fragmentation of information caused by both platforms and the ensuing explosion of grassroots, AI-built applications.

## Project Status: [Alpha]

Tonk is in Alpha. This is a brand spanking new project being built fully in the open, from scratch. Beware, here be dragons! Please [visit our website for more information](https://tonk.xyz) or you may create a post in the [Q&A Discussion](https://github.com/tonk-labs/tonk/discussions/categories/q-a) if you have any questions or need support.

## Features

- **Local-First Architecture**: Built on Automerge for conflict-free data synchronization.
- **Quick Start**: Create new projects instantly with `tonk create app`.
- **Offline Support**: IndexedDB-based storage auto syncs when connectivity resumes.
- **Privacy Focused**: Keeps user data local by default.
- **React + TypeScript**: Modern development stack with full type safety.
- **Tailwind CSS**: Utility-first styling out of the box.
- **Package and Share**: Easily deploy and share your Tonk apps (work in progress).

---

## Getting Started

Run in a terminal

```bash
npm i -g @tonk/cli && tonk hello
```

## AI code writer friendly

The hottest new programming language is English and so we are designing the Tonk framework to plug seamlessly into AI code generation tooling. We think this is incredibly important for two reasons:

1. It allows for rapid app creation to serve constantly changing needs.
2. It makes the power of code available to a new class of programmer.

Tonk shines when it's used with AI! Tonk makes it simple to store and remix your data across applications while limiting the surface area for code generation to where it works best today — the frontend.

#### Claude Code

We find Tonk works best with Claude Code as it's more aggressive with pulling in context. You may install and setup Claude Code [here](https://docs.anthropic.com/en/docs/agents-and-tools/claude-code/overview).

#### Cursor and Windsurf

We've found these LLM-assisted code editors to require more human intervention than Claude Code. They don't make it easy to receive all the relevant context from the project. That means, it's your responsibility to make sure the editor is correctly pulling in the corresponding llms.txt files when it's trying to implement different parts of the code.

## Usage

```
tonk create [app name]
```

### 2. Start the dev tools and open it in your favorite AI-friendly code editor

Tonk is not opinionated about which AI tooling you use.

In the root of the tonk project run `npm run dev`

### Building

When you are happy with your mini-app and ready to use it, you should serve it up and save it to your device as a PWA. This should allow the app to continue working even when you're offline or away from the network.

#### Building Locally

```bash
npm run serve
```

#### AWS Deployment

```bash
# Interactive configuration (with existing instance)
tonk config

# Provision a new EC2 instance
tonk config --provision [--key-name my-key]

# Configure with existing EC2 instance
tonk config --instance ec2-xx-xx-xx-xx.compute-1.amazonaws.com --key ~/path/to/key.pem
```

Options:

- `-i, --instance <address>` - EC2 instance address
- `-k, --key <path>` - Path to SSH key file
- `-u, --user <name>` - SSH username (default: ec2-user)
- `-p, --provision` - Provision a new EC2 instance
- `-n, --key-name <name>` - AWS key pair name (for provisioning)

[Guide to saving PWAs for iOS](https://help.shore.com/en/how-do-i-save-the-pwa-on-my-smartphone)

You can ask the LLM to change the name of the app (which is default to Tonk) in the `public/manifest.json`

---

## Contributions

Because the project is so young, we cannot approve external contributions.

We welcome:

- feedback
- small contributions

_A small contribution is something like a bug-fix or a PR with a diff <50 lines._

#### Become a regular contributor

We are open to regular contributors who will collaborate with us on improving the codebase. If that's you then please [reach out to the team](https://tonk.xyz).

#### How to submit feedback

There will be an issue template you can use. If you do not use the issue template, we will delete your issue.

---

## License

Simplicity and freedom.

MIT © Tonk
