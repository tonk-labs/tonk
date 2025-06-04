# Elevate your creative intelligence

Tonk is a simple toolchain for better vibe coding that helps you build and share applets that integrate with your data and embed into your daily life.

It's for creative people of all stripes - you just need basic coding ability.

AI makes it easier for people to understand the world, but not necessarily their world. Coding copilots make it easier for people to build static websites, but it's still tricky for local experts to embed AI into their idiosyncratic daily lives.

In spite of that, future-facing people with basic coding ability are scraping data, organising files and running it all through LLMs to get smarter, faster and more creative.

Setting up that infrastructure is painful. What if it were simple?

Because we replace a traditional server-database architecture with a local-first approach, Tonk apps are:

- Quick to build
- Easy and cheap to manage
- Private by default
- Work offline
- Guaranteed to stay independent from platforms

<br/>

<div style="display: flex; margin-bottom: 15px;">
<div style="background-color: #FF4500; color: white; padding: 15px; flex: 0.7; text-align: center; border-radius: 5px; display: flex; align-items: center; justify-content: center; margin: 5px">
<strong>Keepsync Library</strong>
</div>
<div style="background-color: #FF4500; color: white; padding: 15px; flex: 0.7; text-align: center; border-radius: 5px; display: flex; align-items: center; justify-content: center; margin: 5px">
<strong>Offline + private<br>functionality</strong>
</div>
<div style="background-color: #FF4500; color: white; padding: 15px; flex: 0.7; text-align: center; border-radius: 5px; display: flex; align-items: center; justify-content: center; margin: 5px">
<strong>Tonk<br>CLI</strong>
</div>
<div style="background-color: #FF4500; color: white; padding: 15px; flex: 0.7; text-align: center; border-radius: 5px; display: flex; align-items: center; justify-content: center; margin: 5px">
<strong>Tonk Daemon</strong>
</div>
</div>
<div style="display: flex; margin-bottom: 15px;">
<div style="background-color: #FF4500; color: white; padding: 15px; flex: 0.7; text-align: center; border-radius: 5px; display: flex; align-items: center; justify-content: center; margin: 5px">
<strong>React + Typescript + Tailwind</strong>
</div>
<div style="background-color: #FF4500; color: white; padding: 15px; flex: 0.7; text-align: center; border-radius: 5px; display: flex; align-items: center; justify-content: center; margin: 5px">
<strong>State<br>management</strong>
</div>
<div style="background-color: #FF4500; color: white; padding: 15px; flex: 0.7; text-align: center; border-radius: 5px; display: flex; align-items: center; justify-content: center; margin: 5px">
<strong>Distributed<br>auth*</strong>
</div>
</div>
<div style="display: flex;">
<div style="background-color: #FF4500; color: white; padding: 15px; flex: 0.7; text-align: center; border-radius: 5px; display: flex; align-items: center; justify-content: center; margin: 5px">
<strong>Share Links</strong>
</div>
<div style="background-color: #FF4500; color: white; padding: 15px; flex: 0.7; text-align: center; border-radius: 5px; display: flex; align-items: center; justify-content: center; margin: 5px">
<strong>Deployment*</strong>
</div>
</div>

<p style="text-align: center;"><em>*under construction</em></p>

## Example usecases

Learning Tonk requires a mindset shift because as developers we're used to thinking in terms of singular applications that are tightly coupled to databases.

Tonk doesn't just help you build apps or store data. Tonk can be best described as helping you build **your own little internet: a way to store shared highly mutable state across many apps and many users, and build ephemeral services, apps and tools on top of that shared state.**

<p style="text-align: center;">
  <img src="./images/illustrative-example.png" style="width: 100%; margin: 4rem auto; background-color: white; padding: 20px; border-radius: 8px;"/>
</p>

Each app can read and write to the shared state with barely any development overhead - no need to worry about migrations, caching, auth or permissions.

Here are some examples of what Tonk can help with:

### 🗺️ Maps

An evolving dataset for your friends to add locations, routes, reviews, planned trips - and surface whatever you like in custom maps apps.

_Hackathon idea:_ A social mapping app where friends can collaboratively pin spots, share routes, and plan trips—with the option to remix the shared dataset into custom experiences like foodie maps, hiking guides, or road trip planners.

### 🎯 Productivity

A fluctuating set of todos for your colleagues to track progress on ephemeral projects without forcing everyone to use the same productivity app.

_Hackathon idea:_ A multiplayer to-do list for temporary teams - where shared project tasks live in a public space and AI nudges contributors to focus on what matters before the project dissolves.

### 💰 Banking

Aggregated financial information for your household to track your finances and make intelligent investments.

_Hackathon idea:_ A household finance dashboard that syncs every member's bank accounts, applies AI to optimize spending, and auto-generates investment proposals based on shared goals.

### ❤️ Health

A dataset for your family to make health data available to bespoke meditation, exercise or sleep apps.

_Hackathon idea:_ A health data layer for families — syncing sleep, steps, and stress scores across devices and enabling personalized wellness bots that work across meditation, exercise, and diet apps.

### 💬 Social

An ad-free, private chatboard for your friends, but where everyone customises their experience with pluggable components such as games, calendars and notifications.

_Hackathon idea:_ A modular group chat app where every conversation is a programmable space - friends can add shared games, calendars, polls, or moodboards, and the feed adapts to how your group vibes, not how the algorithm dictates.

### 🤖 Assistants

An AI that can assist you with full context from your chat apps, calendars, todo boards and social feeds.

_Hackathon idea:_ A privacy-first AI assistant that reads your calendar, chat threads, and todos from your shared spaces — then recommends actions, summarizes life, and shares updates with your friends.

## How it works

Building with Tonk feels like magic because it's default vibecode-friendly, local-first and multiplayer.

Apps generated with the **Tonk CLI** come pre-bundled with React, Typescript, Tailwind and aggressively prompt your agent to ensure smooth vibe-coding.

The **Tonk CLI** provides a simple but powerful command-line interface for interfacing with your Tonk and scaffolding Tonk templates.

The **Tonk** runs in the background on your computer. It hosts your bundles and stores. It's the glue between all your applications.

The **keepsync** library manages state with an optimized sync engine based on Automerge CRDTs, enabling real-time collaboration without a central server.

## Get started

The best place to get started is our [quickstart guide](./quickstart.md).

## Walkthrough Videos

<div style="text-align: center;">
  <div style="padding:62.65% 0 0 0;position:relative;"><iframe src="https://player.vimeo.com/video/1090470142?h=d0f33fc88d&amp;title=0&amp;byline=0&amp;portrait=0&amp;badge=0&amp;autopause=0&amp;player_id=0&amp;app_id=58479" frameborder="0" allow="autoplay; fullscreen; picture-in-picture; clipboard-write; encrypted-media" style="position:absolute;top:0;left:0;width:100%;height:100%;" title="Intro"></iframe></div><script src="https://player.vimeo.com/api/player.js"></script>
  
  <div style="padding:62.79% 0 0 0;position:relative;"><iframe src="https://player.vimeo.com/video/1090470104?h=d9dcca232a&amp;title=0&amp;byline=0&amp;portrait=0&amp;badge=0&amp;autopause=0&amp;player_id=0&amp;app_id=58479" frameborder="0" allow="autoplay; fullscreen; picture-in-picture; clipboard-write; encrypted-media" style="position:absolute;top:0;left:0;width:100%;height:100%;" title="First Vibe Code"></iframe></div><script src="https://player.vimeo.com/api/player.js"></script>
  
  <div style="padding:62.65% 0 0 0;position:relative;"><iframe src="https://player.vimeo.com/video/1090470074?h=7c18bc820a&amp;title=0&amp;byline=0&amp;portrait=0&amp;badge=0&amp;autopause=0&amp;player_id=0&amp;app_id=58479" frameborder="0" allow="autoplay; fullscreen; picture-in-picture; clipboard-write; encrypted-media" style="position:absolute;top:0;left:0;width:100%;height:100%;" title="Cal"></iframe></div><script src="https://player.vimeo.com/api/player.js"></script>
  
  <div style="padding:62.79% 0 0 0;position:relative;"><iframe src="https://player.vimeo.com/video/1090470033?h=5c90389dfc&amp;title=0&amp;byline=0&amp;portrait=0&amp;badge=0&amp;autopause=0&amp;player_id=0&amp;app_id=58479" frameborder="0" allow="autoplay; fullscreen; picture-in-picture; clipboard-write; encrypted-media" style="position:absolute;top:0;left:0;width:100%;height:100%;" title="Agentic"></iframe></div><script src="https://player.vimeo.com/api/player.js"></script>
</div>

## Project status

The team behind Tonk is a venture-backed startup based in London.

The _Tonk toolchain_ is in alpha. This is a brand new project built fully in the open, from scratch. Please ask questions in our community or visit our website for more information.

As an early stage project we are very open to feedback and keen to help builders - so please reach out to the team and we will endeavour to support your usecase.

## Links

- [Github](https://github.com/tonk-labs/tonk)
- [Tonk website](https://tonk.xyz)
- [Telegram community](https://t.me/+9W-4wDR9RcM2NWZk)
