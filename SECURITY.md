# Security policy

kwant runs shell commands, edits files, drives a browser and stores API keys and passwords for you. We take reports about any of that seriously.

## Reporting a vulnerability

**Please don't open a public issue, discussion or pull request for a security problem.**

Report it privately through GitHub instead:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**.
3. Describe the problem, how to reproduce it, which version or commit you tested, and what an attacker could do with it.

Only the maintainers can see the report. We aim to reply within a week, keep you posted while we work on a fix, and credit you in the advisory unless you'd rather stay anonymous.

## Supported versions

kwant is pre-1.0. Security fixes go into `main` and the next release. Older commits and releases don't get backports.

## What counts

In scope, for example:

- A bot getting past an approval: running a command, editing a file or calling a tool that should have needed your OK.
- Reading or leaking kwant's secrets: API keys, CLI logins, the sign-in token, saved passwords or the Bitwarden session.
- One space, bot or bot turn reaching another's data, or a bot's turn token reaching person-only routes.
- Getting past the sign-in on a kwant that listens on a non-loopback address.
- Prompt injection through a teammate's message, a web page, an email or an imported bot that makes kwant act with your authority without an approval.
- Escaping the VM-mode container or the shared browser container.

Out of scope:

- What a bot does after you approved it, or with approvals turned off for that space or bot. That's the setting working as designed.
- Anything that needs someone who already controls your machine or your kwant data folder (`~/.kwnt` by default).
- Fake keys and passwords in tests and fixtures. They are test strings, not real credentials.

## Known limitations

- **Autofill with approvals off.** Saved-password autofill stays on while approvals are off. Chrome's DevTools port listens on loopback without authentication, so a bot with a shell can attach to it and read a field kwant has just filled. This applies to the built-in store and to Bitwarden. The planned fix is to start Chrome with `--remote-debugging-pipe`. Until then, keep approvals on for bots that use saved passwords. See [docs/saved-passwords.md](docs/saved-passwords.md).
- **Sign-in links in the logs.** The `#kwnt_token=…` link that `kwnt serve`, `kwnt signin-link` and `kwnt token` print works once and expires after 15 minutes. Until it's used, anyone who can read the server's output or logs can use it instead of you. You'd then see "used or expired", and their session shows under Settings → Signed-in devices, where you can sign it out. The API token itself (`kwnt token --raw`, `~/.kwnt/api-token`) never expires, so keep it private. Serve kwant over HTTPS when it's reachable from another machine.
- **Approving another device.** Approve a "New device wants access" request only if its code is on a screen you're holding. If someone asks you to approve a code, deny it.
