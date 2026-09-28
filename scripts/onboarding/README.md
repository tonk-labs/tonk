# Bundled Welcome content

The Welcome space is authored in notation, like the rest of the library:

- `onboarding.yaml` holds the vault shell, its Welcome pages, and every page
  entity the sidebar shows. The worker evaluates it after `core.yaml` and
  `onboarding-agent.yaml` when the Welcome space is created.
- `onboarding-demos.yaml` holds the demos (To-Dos, Tonk News, Zork, Budget,
  Dots, Agent playground). The worker evaluates it once Welcome has painted, or
  sooner when navigation needs a demo. It only adds entities of its own, so it
  never replaces an edit made to a Welcome page before it loads.
- `onboarding-media.json` lists the binary blobs the documents refer to (the
  Welcome images and the Zork saves). The worker imports each one on first
  read, after checking its hash, from the file next to it.

Each document's `xyz.tonk.onboarding/imported` marker commits with it, so a
retry never evaluates it again over later edits.

`playground.py` derives `onboarding-agent.yaml` from `core.yaml`. Run it after
changing the agent invitation there.
