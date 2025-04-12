# Tonk Stack

![image of tonk stock](../images/components.png)

The Tonk stack consists of several packages:

---

**CLI**

A command line utility `tonk` that is used to run the GUI, and to run your apps in both dev and serve (or production) mode.

---

**Hub**

The Tonk Hub, which is an Electron app used to assist in running your apps and viewing the state in your stores.

---

**Server**

Runs your apps and listens for changes to stores. Changes are propagated to every client (allowing collaboration) and saved as stores on your machine when you are running your applications with the Tonk CLI or Tonk Hub. The store format for now is simply an automerge binary format.

---

**Keepsync** [docs](keepsync.md)

A Typescript library that wraps any Zustand store and allows it to sync to the server.
