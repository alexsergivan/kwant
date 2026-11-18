title: Control Home Assistant from a self-hosted AI bot
description: Connect Home Assistant to kwant 0.9.21: official MCP server, two-field Connect sheet, the Home team and House, plus what a 7B local model got right and wrong.
date: 2026-11-17
tags: home-assistant, mcp, self-hosted, how-to, ai-agents, kwant
slug: control-home-assistant-with-a-self-hosted-bot
card_title: Control Home Assistant from a self-hosted bot
card: The 0.9.21 walkthrough: Connect sheet, the Home team, and locks that always ask.
card_art: scene
image: assets/og/docs-home-assistant.png
draft: no
---

[kwant](https://github.com/alexsergivan/kwant) is an open-source (AGPL-3.0) team of AI bots you run yourself. I'm the author. This guide is the path that works in **kwant 0.9.21**: Home Assistant's official MCP Server, a two-field **Connect Home Assistant** sheet, the new **Home** team with a bot called **House**, and an approval card before anything risky is changed. Locks, doors, garage doors, gates and alarms always ask, in every approval mode. A shorter reference is in the [Home Assistant docs](/docs/home-assistant).

The short version: enable Home Assistant's Model Context Protocol integration, expose the devices you want the bot to touch, make a long-lived access token, then paste your Home Assistant address and that token into kwant. kwant builds the `/api/mcp` URL and the sign-in header for you. If it can't connect, the sheet says why in plain words.

For House itself, use a **cloud account or a larger local model**. A 7B model (we used Ollama `qwen2.5-home7`, built from `qwen2.5:7b`) is fine for simple, specific asks when you name the room or device — for example "Turn off the living room lamp" or "What is the living room temperature right now?". It struggled with vague asks like "Is anything left on?" or "How warm is it inside?", and once claimed it was unlocking the front door without calling any tool.

This was tested against Home Assistant 2026.11.1 and kwant 0.9.21. The devices in the walkthrough are a demo living-room lamp, kitchen and porch lights, a 21.4 °C temperature sensor and a front-door lock; the same steps apply to your own house.

## What you need

- A running [Home Assistant](https://www.home-assistant.io/) instance you can reach from the machine that runs kwant (same computer or your LAN).
- [kwant 0.9.21](https://github.com/alexsergivan/kwant) or later, installed and open. A Mac app, a Linux desktop or server, or Docker all work.
- An administrator Home Assistant user. The main MCP URL requires an admin account when it exposes the full set of tools.
- An AI account for House. Prefer a cloud model, or a local model larger than 7B, if you want House to handle broad questions and lock-style requests reliably. A 7B Ollama model is enough for short, specific asks that name the room or device. Don't use 3B for this: **qwen2.5 3B** often sent invalid tool arguments and sometimes announced success after a step had already failed.

You do not need Home Assistant to be on the public internet. Everything here stays on your machine or your LAN.

## 1. Enable Home Assistant's MCP server

Home Assistant is the MCP *server* in this setup: your kwant bot is the client that calls its tools. (Home Assistant also has a separate Model Context Protocol *Client* integration, which points the other way. You don't need that here.)

The official integration page is [Model Context Protocol Server](https://www.home-assistant.io/integrations/mcp_server/). On current Home Assistant (2026.11 and later) the integration is named **Model Context Protocol**. Before 2026.11 it was **Model Context Protocol Server**.

You can enable it from **Settings → System → AI**. If the Model Context Protocol card shows **Enable**, select it. You can also add it by hand: **Settings → Devices & services → Add integration**, then choose **Model Context Protocol**.

On a fresh install the main URL exposes all AI tool sets (Home Assistant may title this "All LLM APIs" or "Expose all AI tools"). You can later restrict that under **Configure**.

kwant always connects to the main URL:

```text
http://<your-home-assistant-host>:8123/api/mcp
```

For example, `http://homeassistant.local:8123/api/mcp`. You don't have to type `/api/mcp` into kwant; the Connect sheet adds it.

## 2. Expose the entities the bot should use

Device-control tools (the same ones Assist uses, including `HassTurnOn`, `HassTurnOff` and `GetLiveContext`) only see entities you have exposed to voice assistants.

In Home Assistant, open **Settings → Voice assistants** and use the **Expose** (exposed entities) page. Expose the lights, sensors and other devices you actually want the bot to read or control.

You can leave locks, garage doors, gates and alarms unexposed if you don't want them in the tool list at all. If you do expose them, kwant still always shows an approval card before a bot can unlock, open or disarm them — even with approvals turned off. That card offers no **Always allow**.

If a later prompt fails with "No entities matched", this is the first place to check: the friendly name must be exposed, and the bot must use that name, not the `entity_id` (`light.living_room_lamp` is not the same as `Living room lamp`). A query for the name `temperature` will miss a sensor called **Living room temperature**.

The Connect sheet also notes that you can limit what bots can do under **Model Context Protocol → Configure**. That is a second, coarser filter (which AI tool sets the server offers). Exposing individual entities on the Assist page is still what decides which devices `HassTurnOn` / `HassTurnOff` can see.

## 3. Make a long-lived access token

In Home Assistant, open your profile, then **Security**, then **Long-lived access tokens**, and create a token. Copy it once; Home Assistant will not show it again.

Give it a name you'll recognise, like `kwant`. Treat it like a password: it lets whoever holds it act as you in Home Assistant.

## 4. Connect Home Assistant in two fields

In kwant 0.9.21 you no longer add Home Assistant as a generic MCP server by hand. Open **Connect Home Assistant** from either place:

- **Tools → Find a tool**, then choose **Home Assistant**, or
- Start the **Home** team (see the next section) and use its **Connect Home Assistant** need.

The sheet has two fields:

- **Address** — the address you use to open Home Assistant, like `homeassistant.local:8123` or `http://192.168.1.20:8123`. The placeholder is `homeassistant.local:8123`. Once the address looks valid, the help line reads **kwant will connect to** `http://…:8123/api/mcp`. You can omit the scheme and, on plain http, the port: `homeassistant.local` becomes `http://homeassistant.local:8123/api/mcp`.
- **Access token** — paste the long-lived token. The help text points at **profile → Security → Long-lived access tokens → Create token**. The **Make one in Home Assistant** link opens that Security page once an address is filled in.

Two notes sit under the fields: bots can reach every device unless you limit them in Home Assistant, and locks, doors, garage doors, gates and alarms always ask you first.

The official shot uses `homeassistant.local`. On your own network you'll usually type `homeassistant.local:8123` or Home Assistant's LAN IP.

Press **Connect**. The sheet tests the connection and reports the result in plain words:

| What you see | What it means |
|---|---|
| **Connected · N actions**, with a **Tested** sticker | The server answered and listed its tools. From the Home team, it also says **House can use it now.** From Tools, it says the server is saved and you can start a Home team, or turn it on for any bot in that bot's settings. This test connected with **21 actions**. |
| **Home Assistant didn't accept this access token.** | Make a new token and paste the whole thing. |
| **Home Assistant answered, but its MCP Server is off.** | Add **Model Context Protocol** under **Settings → Devices & services → Add integration**, then try again. The sheet links **Open integrations in Home Assistant**. |
| **kwant can't reach <host>.** | Home Assistant is off, or the address is wrong. If kwant runs on another computer or in Docker, use Home Assistant's IP address, like `http://192.168.1.20:8123`. |
| **This address answered, but it isn't Home Assistant.** | The URL is reachable, but it didn't identify itself as Home Assistant's MCP server. Check the address. |
| **Couldn't connect to Home Assistant.** | Something else went wrong. Check that the address opens Home Assistant in a browser, then try again. |

<figure>
  <div class="frame">{{shot:v0921-home-assistant|The Connect Home Assistant sheet after a successful connect: Address homeassistant.local, "kwant will connect to http://homeassistant.local:8123/api/mcp", Connected · 14 actions with a Tested sticker, and "House can use it now."}}</div>
  <figcaption>Official 0.9.21 product shot from the <a href="{{root}}features.html#connect-home-assistant">features page</a>.</figcaption>
</figure>

The token field clears after every Connect. Leaving it empty on a later try keeps the saved token (the sheet says **Saved. Paste a new one to replace it.**).

## 5. Pick the Home team and meet House

The new **Home** team is the intended starting point. Open **New team** in the sidebar (or the **Get started** card) and look under **From a template**. The Home tile shows **1 bot**.

If you don't have an AI account in this space yet, the sheet says **Add an AI account first**. Add one, then come back to the template.

Open the Home preview and press **Create team · 1 bot**. The preview lists House, what the team needs, and what it will never do without you:

<figure>
  <div class="frame">{{shot:v0921-home-team|The Home team after creation: House as lead, Connect Home Assistant already connected with 14 actions, an optional A local model need, and suggested asks}}</div>
  <figcaption>Official 0.9.21 product shot. The Create team sheet lists Home under From a template (1 bot); this is the team once it exists.</figcaption>
</figure>

That gives you:

- **House** (title: Home Helper), the only bot, with a house avatar and a **Check the house** skill.
- A **Lights out** routine that starts off (see below).
- A required **Connect Home Assistant** need, which opens the same two-field sheet, and an optional **A local model** need. The template's own line says "A small Ollama model is enough." For House, a cloud or larger model is the safer pick; a 7B model is fine only for simple, specific asks.

House is instructed to look at the live state before it changes anything, to name each device and the result in plain words, and never to claim a change it didn't see succeed. It will not unlock a door, open a garage or gate, or turn off an alarm on its own — those always ask you. It also won't change automations, scenes, schedules or settings in Home Assistant.

You can instead connect Home Assistant from Tools and turn the server on for any existing bot. The Home team is just the path that already has the skill, the routine and the right cautions.

## 6. Example asks

Once House is connected and an account can run it, name the room or device. These are the asks that actually worked, with Ollama `qwen2.5-home7` (qwen2.5 7B):

- **Turn off the living room lamp** — House called the turn-off tool with `{"name": "Living room lamp"}`. In the default **Ask only when risky** mode you get an approval card first; after you allow it, the reply was "The living room lamp has been turned off."
- **What is the living room temperature right now?** — House called `GetLiveContext` and answered **21.4 °C**.

Use the friendly names your house uses, not entity ids.

Vague asks were unreliable on this 7B model, even though the Home team suggests them. **Is anything left on?** called `GetLiveContext` with domain `switch` (the test house has lights, not switches) and got `{"error": "No exposed entities found in domain(s): switch"}`, then replied as if nothing was on — while the kitchen and porch lights were on. Adding "Check the lights" still only looked at the living room and answered "The living room light is off." **How warm is it inside?** queried a sensor named `temperature`, missed **Living room temperature**, and answered with outside weather for Berlin instead.

If you want House to handle those broader questions, pin a cloud account or a larger local model.

## 7. Approvals, reads, and locks that always ask

A new space starts on **Ask only when risky**. In that mode:

- Home Assistant's reads run without a card. That includes `GetLiveContext` (and `GetDateTime`) and any other tool the server marks read-only. Home Assistant marks some of those tools both read-only and destructive; since kwant 0.9.20, kwant trusts the read-only mark, as the MCP spec says.
- Turning a light or switch on or off still shows a card. The chat names the device, like **House wants to turn off Living room lamp**.
- You can **Allow once**, or **Always allow** that tool in this conversation (Always allow now lasts after a restart; take it back under the bot's **Allowed without asking**).

**Locks, doors, garage doors, gates and alarms always ask**, in every approval mode, including **Don't ask**. That card offers no **Always allow**, and an earlier Always allow for the same tool (for example one you gave while turning off lights) does not cover them. The card's reason is: *Locks, doors, garage doors, gates and alarms always ask. Always allow isn't offered for them.*

A server counts as Home Assistant by the name it reports when it connects (`home-assistant`), never by its address. Reading the house never asks for this reason.

<figure>
  <div class="frame">{{shot:v0921-locks-ask|An approval card: "House wants to unlock Front door", area Hallway, in Home Assistant, with the lock note and only Deny and Allow once}}</div>
  <figcaption>Official 0.9.21 product shot from the <a href="{{root}}features.html#home-locks-always-ask">features page</a>. Our live test with <code>qwen2.5-home7</code> never called the lock tool, so we didn't capture our own card — see the model notes below.</figcaption>
</figure>

If you walk away, a card waits about five minutes. Nothing runs until you allow it. Since 0.9.21, a routine step nobody approved no longer counts as done: the run says a step was waiting for your approval and didn't run, and the reply's **Details** name the step.

Test approvals on a lamp you don't mind toggling before you point a bot at anything that matters. Then, with a cloud or larger model, try **Unlock the front door** and confirm you get the always-ask card, with only **Deny** and **Allow once**. On 7B, House refused twice, then after an explicit confirmation replied "Unlocking the front door now." with **no tool steps** — the door was not unlocked. Open **Details** before you trust a claim like that.

## 8. The Lights out routine

The Home team comes with a **Lights out** routine. It starts **off** (the Routines page shows it paused). When you turn it on, it runs every day at 22:00 (`0 22 * * *`) and tells House:

> Turn off every light that's still on, then tell me which ones you turned off. Use the Check the house skill.

The skill reads the live state first, turns off only the lights that are on, leaves doors, locks and alarms alone, and never claims a light is off unless Home Assistant said so. On a 7B model, review the first runs carefully: a vague "what's still on?" step can miss lights, as above.

Open the team's routines, review the prompt, then switch Lights out on when you're ready. Watch the first few runs. If a step waits for your approval and you aren't there, 0.9.21 records that it didn't run, instead of marking the routine done.

## 9. Models for House

The Home team's **A local model** need is optional; it opens Accounts so you can add Ollama (or another local endpoint). A local model keeps the house on your computer, but **for House, prefer a cloud account or a larger local model**.

What we actually saw with Ollama:

- **qwen2.5-home7** (qwen2.5 7B) was fine when the ask named the room or device: it turned off the living room lamp and read the living-room temperature as 21.4 °C.
- The same 7B model was not reliable on vague asks, called `GetLiveContext` with the wrong domain or name, and once claimed it was unlocking the front door without calling a tool.
- **qwen2.5 3B** often sent invalid `HassTurnOn` slots (empty domain, underscored entity ids, name "light") and once said the lamp was on after a step had already failed. Don't use 3B for this.

If you do pin 7B, keep the prompts specific, and open **Details** whenever the reply claims a change.

## Troubleshooting

| Symptom | What to try |
|---|---|
| **Home Assistant didn't accept this access token.** | Make a new long-lived token in **Profile → Security** and paste the whole thing. Leaving the field empty keeps a token that was already saved. |
| **Home Assistant answered, but its MCP Server is off.** | Add the **Model Context Protocol** integration, then press **Try again**. A 404 on `/api/mcp` is how this usually shows up. |
| **kwant can't reach <host>.** | Confirm the address opens Home Assistant in a browser from the same machine that runs kwant. From Docker or another computer, use an IP address, not `.local`. |
| **This address answered, but it isn't Home Assistant.** | You're not hitting HA's MCP server. Check the address; don't point it at a different app's `/api/mcp`. |
| The server connects but House says it cannot find a device, or **No entities matched** | Expose the entity on the Assist / Expose page. Use the friendly name, not the entity id. |
| **No exposed entities found in domain(s): switch** | The model asked for switches. Name a light (or the room) in the prompt, or pin a cloud / larger model. |
| **No exposed entities matched name 'temperature'** | The sensor's friendly name is more specific (ours is **Living room temperature**). Name that, or use a larger model. |
| **Received invalid slot info** | The model sent empty or underscored slots. Use a larger or cloud model, and ask it to pass only `"name"` with the exact friendly name. |
| **Details · a step failed**, but the bot claims the light is on | Open **Details** and read the tool result. A failed first call can sit next to a later successful one, or next to a guess. Don't trust the prose alone. Since 0.9.21, Details name which step failed or didn't run. |
| The bot says it is unlocking or turning something off, but Details shows no steps | Nothing ran. The 7B House bot did this after a confirmation to unlock the front door. Don't trust the claim; pin a cloud or larger model. |
| A lock, door, garage, gate or alarm ran without a card | It shouldn't, in any approval mode, if a tool actually ran. Check **Details**. A sentence with no tool call is a model claim, not a completed action, and is not this always-ask card. |
| A routine runs but does not turn lights off | Read the routine reply and **Details**. If it says a step was waiting for your approval, allow the card or tighten the prompt. Confirm the lights are exposed. |
| **No account can run bots here yet** / **Add an AI account first** | Add or re-enable Ollama (or another account) under **Settings → This space → Accounts**. |
| **The model request failed: connection failed** or `model '…' not found` | Ollama is down, or the pinned model name no longer exists. Check `ollama list` and the account in that space. |

## Advanced: adding Home Assistant as a generic MCP server

You shouldn't need this in 0.9.21. The Connect sheet is the supported path: it builds the `/api/mcp` URL and the `Authorization: Bearer <token>` header, and it checks that the server really is Home Assistant.

If you do add a server by hand under Tools, use the same URL and a header `Authorization` = `Bearer <token>`, then **Test connection**. Don't use OAuth against a plain-http LAN address such as `http://homeassistant.local:8123/api/mcp`. kwant only allows http OAuth on loopback (`localhost` / `127.0.0.1`). On a `.local` or `.lan` hostname, sign-in fails with a message that the server's sign-in must use https, and that for a server on your home network you should add the Bearer header instead.

## Disclaimer

I am the author of kwant. The Home Assistant MCP server is Home Assistant's official integration; this guide only shows how to connect it to a self-hosted kwant bot, including on local models. Test approvals on a lamp you don't mind toggling before you point a bot at anything that matters.
