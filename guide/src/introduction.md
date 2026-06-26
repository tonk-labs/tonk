# Introduction

Tonk is a personal information substrate. At its heart is Dialog, a distributed database built around local-first principles: your data lives with you, works offline, and syncs directly between peers without a central server. Access is granted with [UCANs](https://github.com/ucan-wg/spec), capability tokens you mint and delegate.

It feels like Git crossed with a database. Like Git, your data is versioned, content-addressed, and held in repositories you branch and share. Like a database, you query and relate it freely through a small declarative language.

On top of it you define applications declaratively: **concepts** (how you model data as relations), **views** (how it renders), and **behaviors** (how it responds to interaction). They keep working offline and reconcile automatically when peers reconnect.

The fastest way to understand this is to build something. The [next chapter](./example.md) builds a counter one piece at a time. The chapter after that names the model behind it. If you have the CLI, `tonk guide` prints the full notation reference and `tonk schema` lists what is on your branch.
