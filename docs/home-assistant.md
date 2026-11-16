# Home Assistant

kwant connects to [Home Assistant](https://www.home-assistant.io) through Home Assistant's own
MCP server, the [Model Context Protocol integration](https://www.home-assistant.io/integrations/mcp_server/).
Bots can then read the state of your home and switch things, and every change follows your
approvals. Locks, doors, garage doors, gates and alarms always ask.

## What you need

- **Home Assistant 2025.2 or later**, with its Model Context Protocol integration added. In
  Home Assistant, open **Settings › Devices & services › Add integration** and pick **Model
  Context Protocol**. Before 2026.11 it is listed as **Model Context Protocol Server**.
- **A long-lived access token.** In Home Assistant, open your profile, then **Security ›
  Long-lived access tokens › Create token**, and copy it: Home Assistant shows it once. kwant uses
  this token, not Home Assistant's sign-in page.
- **An address kwant can reach**, the one you open Home Assistant at, such as
  `homeassistant.local:8123`, `http://192.168.1.20:8123` or your `https://…` remote address.

Home Assistant's own setup steps, options and limits are on its
[integration page](https://www.home-assistant.io/integrations/mcp_server/). Two of its options
change what your bots get, both under **Settings › Devices & services › Model Context Protocol ›
Configure** (Home Assistant 2026.10 and later):

- **Expose all AI tools** is on for a new setup. Then the address kwant uses (Home Assistant's
  **Main URL**, `/api/mcp`) also offers the **Home Assistant** set of tools, which can read every
  entity and run any action. To give bots only the devices you expose to Assist
  (**Settings › Voice assistants › Expose**), turn it off and pick only **Device control**
  (called Assist in older versions).
- **Require an administrator account** is on for a new setup. Then the token has to belong to an
  administrator, or Home Assistant refuses it and kwant shows the token error below.

## Connect in kwant

The same **Connect Home Assistant** sheet opens from two places:

- **The Home team.** Its chat shows what the team needs; press **Connect** on
  **Connect Home Assistant** (or **Fix** after a failed check).
- **Tools.** Open **Settings › This space › Apps & accounts › Advanced › Manage tools › Add
  server**, then **Home Assistant** under **Home** (or search for "Home Assistant" or "hass"). With
  no tool server yet, there is no **Manage tools**: use **+ Add connection › Tool server (MCP)**
  instead.

The sheet has two fields:

| Field | What to put there |
|---|---|
| **Address** | Where you open Home Assistant. With no `http://` or `https://`, kwant uses `http://`; plain `http` with no port gets Home Assistant's `8123`. Any path is replaced: kwant connects to `<address>/api/mcp`. |
| **Access token** | The long-lived access token. Once the address is in, **Make one in Home Assistant** next to the label opens your profile's Security page. kwant sends it as `Authorization: Bearer <token>` and stores it masked, like every header (see [mcp-api.md](mcp-api.md)). |

Press **Connect**. kwant saves the server in Tools (named `home-assistant`, or `home-assistant-2` and so on if that name is
taken), checks it, and shows
**Connected · N actions** when Home Assistant answers and lists what it can do. From the Home team,
kwant gives it to the team's bots, so House can use it at once. From Tools, it is saved in Tools:
start a Home team (**Start a Home team**, shown while you have none), or turn it on for any bot in
that bot's settings.

The token field empties as soon as you press Connect. If the check fails after the save, leaving
the field empty on **Try again** keeps the saved token, so you can fix only the address.
Closing the sheet after a failed check removes the server it just saved, so nothing broken stays
in Tools.

When the check fails, the sheet says what happened and what to do:

| The sheet says | What to do |
|---|---|
| Home Assistant didn’t accept this access token. | Make a new token in Home Assistant and paste all of it. With **Require an administrator account** on, the token must be an administrator's. |
| Home Assistant answered, but its MCP Server is off. | Add the Model Context Protocol integration in Home Assistant (**Open integrations in Home Assistant**), then **Try again**. |
| kwant can’t reach `<host>`. | Check that Home Assistant is on and the address is right. If kwant runs on another computer or in Docker, `homeassistant.local` may not resolve there: use Home Assistant's IP address, like `http://192.168.1.20:8123` |
| This address answered, but it isn’t Home Assistant. | The server answered under another name. Use the address you open Home Assistant at. |
| Couldn’t connect to Home Assistant. | Check that the address opens your Home Assistant in a browser, then try again. The server's own message is shown under it. |

## The Home team

**New team › From a template › Home** makes:

- **House**, the lead and only bot. It answers what is on, open or warm, turns things on and off
  when you ask, and names each device and result. It never unlocks a door, opens a garage door or
  gate, turns off an alarm on its own, or changes automations, scenes or schedules.
- **Check the house**, House's skill: one short list of what is on, open or running.
- **Lights out**, a routine every day at 22:00 that turns off the lights still on and says which.
  It is **off** until you turn it on.

The team needs Home Assistant (required) and, optionally, a local model to keep what happens at
home on your own computer.

## What asks and what doesn't

Home Assistant's tools follow the bot's [approvals](approvals.md), with one rule on top:

- **Reads.** Under **Ask only when risky**, tools Home Assistant marks read-only (its
  `readOnlyHint`), like looking at the live state of the home, run without a card. Under
  **Ask before running commands**, they ask like every tool.
- **Switching things** (lights, heating, blinds, media) asks or runs as your approvals mode says.
  **Always allow** on such a card lets that bot run that tool without a card in that conversation
  only, and it still holds after kwant restarts. **Bot settings › Approvals › Allowed without
  asking** lists these, with **Take back** on each, and **Take back all** when there are two or more.
- **Locks, doors, garage doors, gates and alarms always ask**, in every mode, approvals off
  included. A call counts when its arguments name the `lock` or `alarm_control_panel` domain, a
  `cover` that is a garage, door or gate (or doesn't say), or contain the word lock, unlock, door,
  garage, gate, alarm or disarm. The card says "Locks, doors, garage doors, gates and alarms always
  ask. Always allow isn’t offered for them." Allowing it covers that one call. kwant applies this
  to any server that calls itself `home-assistant` when it connects, and to a server it has not
  heard from yet; see [approvals.md](approvals.md).

## Tested

kwant was tested against its own Home Assistant-shaped test server, not against every Home
Assistant setup.
