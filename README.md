```
   ・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜

     ____  ____  _  _  Oooo. ____  _____  _____  ____
    (_  _)(_  _)( \( ) ( Y )( ___)(  _  )(  _  )(_  _)
⚡     )(   _)(_  )  (   ) /  )__)  )(_)(  )(_)(   )(     ⚡
     (__) (____)(_)\_) (_/  (__)  (_____)(_____) (__)

   ・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜
```

**Tinyfoot** is an AI-driven developer framework for building highly personal software.

In tandem with **Tinyfoot**, Tonk is building a **maximally interoperable network** designed to address the fragmentation of information caused by the explosion of grassroots, AI-built applications.

## Project Status: [Alpha]

Tinyfoot is in Alpha. This is a brand spanking new project being built fully in the open, from scratch. Beware, here be dragons! Please [reach out to the team](https://linktr.ee/tonklabs) or you may create a post in the [Q&A Discussion](https://github.com/tonk-labs/tinyfoot/discussions/categories/q-a) if you have any questions or need support.

## Features

- **Local-First Architecture**: Built on Automerge for conflict-free data synchronization.
- **Quick Start**: Create new projects instantly with `create-tinyfoot-app`.
- **Offline Support**: IndexedDB-based storage with automatic sync.
- **Privacy Focused**: Keeps user data local by default.
- **React + TypeScript**: Modern development stack with full type safety.
- **Tailwind CSS**: Utility-first styling out of the box.
- **Package as PWA (in progress)**: Packaging as a PWA means your webapp continues to work as desired.

---

## Getting Started

Run in a terminal

```bash
npx @tonk/create-tinyfoot-app my-first-app
```

#### Claude Code

If you are using Claude Code, then there is nothing you need to do. Just run

```
claude .
```

in the Tinyfoot app directory.

#### Cursor and Windsurf

We've found these LLM-assisted code editors to require more human intervention than Claude Code. They don't make it easy to receive all the relevant context from the project. That means, it's your responsibility to make sure the editor is correctly pulling in the corresponding llms.txt files when it's trying to implement different parts of the code.

## Usage

This will generate tinyfoot application boilerplate for you.

### 2. Start the dev tools and open it in your favorite AI-friendly code editor

Tinyfoot is not opinionated about which AI tooling you use.

In the root of the tinyfoot project run `npm run dev`

### Building (unstable)

When you are happy with your mini-app and ready to use it, you should serve it up and save it to your device as a PWA. This should allow the app to continue working even when you're offline or away from the network.
[Guide to saving PWAs for iOS](https://help.shore.com/en/how-do-i-save-the-pwa-on-my-smartphone)

Just run the command

```bash
npm run serve
```

You can ask the LLM to change the name of the app (which is default to Tinyfoot) in the `public/manifest.json`

NOTE: still some issues with this step, we're working on stabilizing it.

---

## Contributions

Because the project is so young, we cannot approve external contributions.

We welcome:

- feedback
- small contributions

_A small contribution is something like a bug-fix or a PR with a diff <50 lines._

#### Become a regular contributor

We are open to regular contributors who will collaborate with us on improving the codebase. If that's you then please [reach out to the team](https://linktr.ee/tonklabs).

#### How to submit feedback

There will be an issue template you can use. If you do not use the issue template, we will delete your issue.

---

## License

Simplicity and freedom.

MIT © Tonk
