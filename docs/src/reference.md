# Reference

We encourage you to look into the llms.txt files in any of your app project files to get more context on how a Tonk app is laid out and how to properly use and configure keepsync.

# FAQ

## Pre-requisites to install

1. You'll need to have wget and the right build tools (from VSCode or from Xcode) available to build the Tonk installation

## How do I get it working on Windows?

1. Do a wsl install (https://learn.microsoft.com/en-us/windows/wsl/install)

2. Install node and npm
   curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.2/install.sh | bash

3. Install necessary shared libraries
   sudo apt install libnss3
   sudo apt install libasound2t64

then copy and paste the install command listed on the [tonk.xyz](https://tonk.xyz) website.

## How do I make changes to my application?

If you are using Cursor or Vscode, just click on the app and run the command in the terminal (you will need to make sure VSCode has installed the command line tool into your path).

```
code .
```

If you are using Claude Code, then run command

```
claude
```

if you are using Windsurf on Mac, then run the command

```
/Applications/Windsurf.app/Contents/MacOS/Electron .
```

When you click on an application, you can run the command inside the terminal `tonk dev`. Then you can open a browser at "localhost:3000".

As you make changes in the editor, you should see changes live in the browser. Tonk apps when running 'tonk dev' use hot reloading.

## How do I run code that is private or that can hit external APIs outside the browser?

When you create a new Tonk app, you should see a server/ folder in the app. There are instructions in the llms.txt in that directory on how to use it. The server/ directory is an express app that runs on your local machine and all Tonk apps can hit on the /api endpoint. This allows Tonk apps to more easily hit external services, use private API_KEYs, or fetch data off your local machine.

This is a new feature, so if it doesn't exist, chat with us and we'll help you to get it setup.

```

```
