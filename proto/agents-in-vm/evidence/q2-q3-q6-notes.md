# Spike notes not covered by adversarial.sh (run by hand 2026-11-08)

Q2 Codex 0.162.1
- Ignores OPENAI_BASE_URL (went to wss://api.openai.com). Works with a custom provider in CODEX_HOME/config.toml.
- API key: [model_providers.kwnt] base_url=http://127.0.0.1:8080/openai/v1, env_key=OPENAI_API_KEY (dummy), wire_api="responses"
  -> full turn, upstream logged openai_key_ok with the real key.
- ChatGPT sign-in: dummy auth.json (unsigned JWT id/access tokens, account acct-dummy) +
  [model_providers.kwntgpt] base_url=".../chatgpt/backend-api/codex", requires_openai_auth=true, wire_api="responses",
  chatgpt_base_url=".../chatgpt/backend-api/" -> full turn, upstream got real bearer + chatgpt-account-id acct-canary (chatgpt_ok).
- Broker denied codex side calls: plugins, wham/accounts/check, wham/settings/user, analytics-events, ps/mcp. Turn still completed.
Q2 Grok 1.0.62
- API key: XAI_API_KEY dummy + GROK_MODELS_BASE_URL + GROK_CLI_CHAT_PROXY_BASE_URL -> /xai/v1: upstream saw the real
  bearer on /models, /chat/completions, /responses. Mock speaks no xAI format, so no full turn.
- Sign-in: GROK_AUTH_PROVIDER_COMMAND="echo kwnt-dummy-session" -> dummy bearer sent to the overridden base for
  /user, /settings, /models, /bundle/archive, /subagents/bundle (all denied by the allow-list). Inference path not reached.
Q3
- --network none + bind-mounted unix socket + in-container socat: no DNS, no IP route, no host, forwarder 403s others.
- --internal network (kwnt-proto-lin-int): no DNS (rc 6), 1.1.1.1 rc 7, BUT the bridge gateway 172.19.0.1:22 was OPEN,
  so every host service bound to 0.0.0.0 is reachable. Rejected.
Q6
- codex sandbox -P :workspace / :read-only on this host: "bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted"
  (Ubuntu 24.04 apparmor_restrict_unprivileged_userns=1). Codex's Linux sandbox cannot start here at all.
- claude 2.1.316 binary has sandbox keys denyRead/allowRead/denyWrite/allowWrite/allowUnsandboxedCommands/
  autoAllowBashIfSandboxed/enableWeakerNestedSandbox. Not run (no bwrap on host).
