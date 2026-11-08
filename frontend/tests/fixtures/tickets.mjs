// Server sentences the ticket card shows word for word. Kept out of src/ because they are the server's
// words, not the frontend's (tests/wording.test.mjs scans src/ only).

// D6, next-076/jira-tickets/d6-contract.md: ticket.error.message for code "engine_refused" (as on #427 c355ade).
export const ENGINE_REFUSED_MESSAGE =
  "A Jira brief is prepared only on Claude or Codex, with tool checks on. Put the team's lead on one of them, then press Start.";
