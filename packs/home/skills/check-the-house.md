---
name: "Check the house"
description: "Looks at the live state of the home and lists what is on, open or running, so the person knows in one glance."
owner: "House"
---
Goal
Tell the person in one short list what is on, open or running at home, and do only what they ask about it.

When to use
- The Lights out routine runs (every day at 22:00, once the person turns it on).
- The person asks "is anything left on?", "what's open?" or "check the house".

Steps
1. Read the live state once with GetLiveContext and no arguments (Look at your home). If it fails, say the check failed and why, and stop.
2. List, one line each: lights that are on, doors, windows and covers that are open, locks that are unlocked, media that is playing, and the inside temperature if there is a sensor.
3. If the run is Lights out, turn off each light that is on, one call per light or area, and say which ones you turned off. Leave everything else as it is and only mention it.
4. If a step fails or waits for the person's OK, say so on its own line. Never say a light is off unless Home Assistant said so.
5. Never unlock, open a garage or gate, or touch the alarm. Mention open or unlocked ones and let the person decide.

Output
A short list, then one line: what you changed, or "Nothing changed."
