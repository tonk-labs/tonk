# Reference

We encourage you to look into the llms.txt files in any of your app project files to get more context on how a Tonk app is laid out and how to properly use and configure keepsync.

# FAQ

## Pre-requisites to install

1. You'll need to have wget and the right build tools (from VSCode or from Xcode) available to build the Tonk installation

## How do I run code that is private or that can hit external APIs outside the browser?

When you create a new Tonk app, you should see a server/ folder in the app. There are instructions in the llms.txt in that directory on how to use it. The server/ directory is an express app that runs on your local machine and all Tonk apps can hit on the /api endpoint. This allows Tonk apps to more easily hit external services, use private API_KEYs, or fetch data off your local machine.

This is a new feature, so if it doesn't exist, chat with us and we'll help you to get it setup.
