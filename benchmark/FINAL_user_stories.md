# Richer User Stories

## Story 1: Marcus (Senior Engineer, power LLM user)

Heavy Claude Code user with strong git/CLI skills. Has rules and patterns set up under an ever-growing file called [CLAUDE.md](<http://CLAUDE.md>) - is thinking about turning this 'universal architecture' file into something to share across projects, platforms, and domains (personal/professional).

**Data to include**

* Soul and heartbeat: brain of agent
* Architecture, specs: project specific
* Newest docs: e.g. [Context7](<https://context7.com/>)
* Style guides: e.g. certain way I like to work with Typescript

> They might be willing to set up a new 'thing' but they have existing strong opinions (about agent orchestration etc., cf. gas town).
>
> Not the project content, it's meta prompts (development environment)

**Why is this a priority?**

* Need parity with existing solutions (we just need to know if it works) **\[CA-03\]**
* Value here (right now): [large contexts make LLMs really slow and expensive](<https://www.producttalk.org/context-rot/?srsltid=AfmBOoo76qpo5I4TtbtDFF9miWvK8Nb46DbTW2onfB6CWwTCej-L1BC8>) (context rot); queryable data is more efficient — e.g. "oh I'm writing Typescript here, I wonder if there is a style guide in the Carry"
* Value here in the future: you can see the diff (history) for your setups
* Power LLM users are our early adopters, they need to be happy

**Open Questions**

* How are they reused over time? 1 project evolving vs 1 setup across multiple projects.

**Focus: LLM efficiency, development environment**

### Story 1.1 Daniel (Frontend Developer, intermediate LLM user) - included but not a priority

Pointed to a skill to do web animations by a friend. The [SKILL.md](<http://SKILL.md>) abstraction is exactly right for him: someone else sets it up, he just invokes it. He uses it for his coding projects.

* Can someone paste a link into an LLM and the LLM can translate into Carry
* This is Story 1, but assume user has less insight into what's going on and a harder time recovering from mistakes.

---

## Story 2: Keri (Independent Consultant, intermediate LLM user)

Wants to build a semantic user profile of herself (expertise areas, communication style, output preferences, active projects) as typed facts in Carry (she's a graph database fan). The graph is richer than any flat "memory" entry; she can share it with any agent and maintain it over time. The unlock is that her professional identity is now portable across tools.

**Data to include**

* Assertions?
  * Unclear how to start *(needs more research!)*
  * Make a fake profile here 
* World model? Creating models over unstructured text/data. Directing LLM behavior!
  * Goblin's wiki on Tonk
  * 2026 world events
  * Ade teaches his LLMs essays and articles
  * Chris's TTRPG
* Personal data container, assistant functionalities
  * Load up personal data (email, calendar, notes) and restrict LLM context to what it needs

> Could attach signifiers to the data and determine how LLMs interacts with it, e.g. "work" and "hobby" and other ways to slice up the data

**Why is this a priority?**

* Value here is giving agents selective information based on context and trust
* Value here is discoverability (user knows what information agents have accessed)
* Value here (potentially) is information hierarchy for agents (e.g. age, profession, location are more foundational), AKA meta structure on the information (*metacircular*) AKA "context on context"

**Focus: expressibility and data ownership**

---

## Story 3: Joppe (Community Archivist, local-first advocate) - included but not a priority

Migrates an Obsidian vault into Carry to build a queryable community knowledge base, which gets updated from a Discord bot he set up. These are mostly links that people share. Doesn't quite know what to query yet but it's fun to have all the knowledge in the community *mapped*, even if it's just local to his machine.

* Fun exploration, generate new insights
  * How are my online communities different (compare multiple vaults)
* Similar to Story 2, but more "soupy" — meaning data is unstructured and LLM/Carry has to learn how to use it.

**Data to include**

* Database of links (cf. User + Agents)
* Research papers
  * We could hide a signal inside the database and see if the LLM can find the salient 