# Tonk Stack

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

The Tonk stack consists of a few packages:

---

**Tonk CLI**

A command line utility `tonk` that runs the Tonk Daemon and communicates to your Tonk. This tool helps you to create Tonk template applications and manage different packaged bundles you are hosting.

---

**Tonk Daemon**

Hosts app bundles, the sync websocket and saves changes of stores to the filesystem. Changes are propagated to every client (allowing collaboration) and saved as stores on your machine when you are running your applications connected with keepsync. The store format for now is simply an automerge binary format.

---

**Keepsync** [docs](keepsync.md)

A Typescript library that wraps any Zustand store and allows it to sync to the server.
