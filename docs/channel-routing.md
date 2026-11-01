# Channel routing

A message in a channel without an @mention goes to the member that fits it best. Code:
`src/router/team.rs` (scoring), `src/server/concierge.rs`, `src/server/rerouting.rs`.

## Fit

Each pick has a `fit` from 0 to 1, saved on the reply as `meta.routing.fit`.

| Fit | What happens |
|---|---|
| ≥ 0.6 | Routed to the best member |
| 0.4–0.6, two candidates | The person is asked (basis `ask`), in any channel. Never for @mentions or DMs |
| < 0.4 in #general | The Concierge answers (basis `concierge`), made on first need |
| < 0.4 elsewhere | The channel's own members, as before (lead or last speaker) |

Laya's titles-only guess is a tiebreaker only; it no longer picks a bot on its own.

## covers / avoids

`GET /api/kwants`, `GET /api/kwants/:id` and `PUT /api/kwants/:id` carry `covers: [string]` and
`avoids: [string]` (at most 20 entries of 60 characters; trimmed, duplicates dropped ignoring
case). On `PUT`, a list left out stays as it is and `[]` clears it. A covered entry counts double
in scoring; an avoided entry keeps the bot out unless it is @mentioned. Empty lists change nothing.
Only the Concierge gets defaults.

## Asking

`POST /api/chat/send` returns `{status: "asking", user_message_id, route_choice_id,
card_message_id, choices: [{kwant_id, name, title}], general: {label, name, kwant_id}}` and posts a
system card with `meta.kind = "route_choice"`. No turn runs until the person chooses:

`POST /api/route-choices/:id/resolve` with `{kwant_id}` (one of the offered bots) or
`{general: true}` (the Concierge in #general, else the channel lead). Person only (403 for bot
tokens); 404 unknown, 409 already chosen, 400 for neither/both or a bot that wasn't offered.
Returns `{status: "dispatched", user_message_id, kwant, kwant_id}`; the event
`route_choice_updated {choice}` carries the updated card meta.

## Hand-back

A routed bot can call `hand_back {reason}` ("not my area"). The message goes on once (basis
`handback`, with `handed_back_by`) to the next member that fits (fit ≥ 0.6), else the Concierge,
else the lead. A message takes at most one hop; a second `hand_back` is refused.
