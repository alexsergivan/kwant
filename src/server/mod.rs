pub mod discussion;
pub mod routes;
pub mod sse;
pub mod browser_stream;
pub mod mcp_oauth;
pub mod mcp_routes;
pub mod mcp_runtime;
pub mod profile_routes;
pub mod profile_turn;
pub mod engine_routes;
pub mod engine_login;
pub mod auth;
pub mod signin;
pub mod continuity;
pub mod model_usage;
pub mod budget;
pub mod scheduler;
pub mod turns;
pub mod delegation;
pub mod parked;
pub mod attachments;
pub mod questions;
pub mod activity;
pub mod human_steps;
pub mod skills;
pub mod spaces;
pub mod asks;
pub mod away;
pub mod computer;
pub mod mcp_suggest;
pub mod page_source;
pub mod connection_suggest;
pub mod bot_actions;
pub mod hires;
pub mod concierge;
pub mod rerouting;
pub mod import_routes;
pub mod approval_modes;
pub mod passwords;
pub mod loose_ends_owner;
pub mod zombie_routes;
pub mod parallel_routes;
pub mod document_routes;
pub mod connections;
pub mod setup_routes;
pub mod push;
pub mod waiting;
pub mod decide;
pub mod work_ahead;
pub mod missions;
pub mod app_update_routes;

use crate::broker::PermissionBroker;
use crate::config::Config;
use crate::db::Database;
use crate::drivers::events::RuntimeEvent;
use crate::router::LayaRouter;
use crate::sandbox::{BrowserController, SandboxManager};
use crate::tools::ToolRunner;
use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Database,
    pub tools: ToolRunner,
    pub broker: PermissionBroker,
    pub router: LayaRouter,
    pub event_sender: broadcast::Sender<RuntimeEvent>,
    /// Running dispatches (their cancel tokens) and turns, and the caps on concurrent turns.
    pub turns: turns::TurnRegistry,
    pub sandbox_manager: Arc<SandboxManager>,
    pub browser_controllers: Arc<Mutex<HashMap<String, BrowserController>>>,
    pub browser_service: Arc<crate::sandbox::service::BrowserService>,
    /// Live claude CLI turns whose MCP permission prompts kwnt answers, by URL token.
    pub mcp_sessions: Arc<Mutex<HashMap<String, mcp_runtime::PermissionSession>>>,
    pub mcp_oauth: Arc<mcp_oauth::OAuthState>,
    /// LLM profiles: sign-in checks and in-app logins.
    pub profiles: Arc<crate::profiles::ProfileManager>,
    /// Which engines can run and their models: checked at startup and on Re-check.
    pub engines: Arc<crate::drivers::engines::EngineRegistry>,
    /// API token for requests from other machines (see `auth`).
    pub api_token: Arc<auth::ApiToken>,
    /// Live turns that have the agents' own tools, by bearer token.
    pub self_tools: Arc<Mutex<HashMap<String, continuity::SelfScope>>>,
    /// Routines with a run in progress; a run that would overlap is skipped.
    pub running_routines: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Wakes the scheduler early when routines change.
    pub scheduler_wake: Arc<tokio::sync::Notify>,
    /// `ask_user` calls waiting for the user's answer.
    pub questions: Arc<questions::QuestionHub>,
    /// `request_human_step` calls waiting for the person to sign in or solve a CAPTCHA.
    pub human_steps: Arc<human_steps::HumanStepHub>,
    /// "Sign in with Claude" for the Claude engine.
    pub claude_login: Arc<engine_login::ClaudeLogin>,
    /// The MCP Registry agents search and suggest servers from (`KWNT_MCP_REGISTRY_URL`).
    pub mcp_registry: Arc<crate::mcp::registry::McpRegistry>,
    /// Composio's app catalogue (`KWNT_COMPOSIO_API_URL`), called only once Composio is added.
    pub composio_api: Arc<crate::mcp::composio::ComposioApi>,
    /// Saved website passwords, outside every bot's folder (see `sandbox::credentials`).
    pub credentials: Arc<crate::sandbox::vault::PasswordVault>,
    /// Zombie Mission Hunter: one scan at a time, and the last scan's report.
    pub zombie: Arc<crate::zombie::hunt::Hunter>,
    /// Parallel Universe: one run at a time, and the harness it uses.
    pub parallel: Arc<crate::parallel::run::Runner>,
    /// Camera-to-Completed: one letter read at a time (and a test's stand-in model).
    pub documents: Arc<crate::documents::extract::Desk>,
    /// The Setup checklist's install jobs (one at a time).
    pub setup: Arc<setup_routes::SetupJobs>,
    /// Web Push to the person's installed app: the VAPID key and subscriptions.
    pub push: Arc<push::Push>,
    /// The Mac app's in-app update; unsupported (and never changing) anywhere else.
    pub app_update: Arc<crate::app_update::UpdateHub>,
}

impl AppState {
    pub fn new(
        config: Config,
        db: Database,
        tools: ToolRunner,
        broker: PermissionBroker,
        router: LayaRouter,
    ) -> Self {
        let (event_sender, _) = broadcast::channel(128);
        let config_data_dir = config.data_dir.clone();
        let sandbox_manager = Arc::new(SandboxManager::new());
        let browser_controllers = Arc::new(Mutex::new(HashMap::new()));
        let profiles = Arc::new(crate::profiles::ProfileManager::new(config.data_dir.clone()));
        let api_token = Arc::new(auth::ApiToken::load(&config));
        let browser_service = Arc::new(crate::sandbox::service::BrowserService::new(config.data_dir.clone(), config.browser_cdp_url.clone()));
        Self {
            config,
            db,
            tools: tools.with_sandbox(sandbox_manager.clone()),
            broker,
            router,
            event_sender,
            turns: turns::TurnRegistry::default(),
            sandbox_manager,
            browser_controllers,
            browser_service,
            mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
            mcp_oauth: Arc::new(mcp_oauth::OAuthState::default()),
            profiles,
            engines: Default::default(),
            api_token,
            self_tools: Default::default(),
            running_routines: Default::default(),
            scheduler_wake: Default::default(),
            questions: Default::default(),
            human_steps: Default::default(),
            claude_login: Default::default(),
            mcp_registry: Arc::new(crate::mcp::registry::McpRegistry::from_env()),
            composio_api: Arc::new(crate::mcp::composio::ComposioApi::from_env()),
            credentials: Arc::new(crate::sandbox::vault::PasswordVault::open(&config_data_dir)),
            zombie: Default::default(),
            parallel: Default::default(),
            documents: Default::default(),
            setup: Default::default(),
            push: Arc::new(push::Push::load(&config_data_dir)),
            app_update: crate::app_update::UpdateHub::shared(),
        }
    }

    pub fn with_sandbox_manager(mut self, manager: Arc<SandboxManager>) -> Self {
        self.sandbox_manager = manager.clone();
        self.tools = self.tools.with_sandbox(manager);
        self
    }
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/auth/status", get(auth::status))
        .route("/api/spaces", get(spaces::list).post(spaces::create))
        .route("/api/spaces/:id", get(spaces::get).patch(spaces::update))
        .route("/api/auth/session", post(auth::create_session).delete(auth::delete_session))
        .route("/api/signin/links", post(auth::create_link))
        .route("/api/auth/device", post(auth::device_start))
        .route("/api/auth/device/poll", post(auth::device_poll))
        .route("/api/signin/requests", get(auth::device_requests))
        .route("/api/signin/requests/approve", post(auth::device_approve))
        .route("/api/signin/requests/deny", post(auth::device_deny))
        .route("/api/signin/sessions", get(auth::sessions))
        .route("/api/signin/sessions/:id", delete(auth::revoke_session))
        .route("/api/push/key", get(push::key))
        .route("/api/push/subscribe", post(push::subscribe))
        .route("/api/push/unsubscribe", post(push::unsubscribe))
        .route("/api/app/update", get(app_update_routes::get_status))
        .route("/api/app/update/check", post(app_update_routes::check))
        .route("/api/app/update/install", post(app_update_routes::install))
        .route("/api/profiles", get(profile_routes::list).post(profile_routes::create))
        .route("/api/profiles/:id", axum::routing::patch(profile_routes::update).delete(profile_routes::remove))
        .route("/api/profiles/:id/key", put(profile_routes::set_key).delete(profile_routes::delete_key))
        .route("/api/profiles/:id/check", post(profile_routes::check))
        .route("/api/profiles/:id/login", post(profile_routes::start_login))
        .route(
            "/api/profiles/:id/login/:flow_id",
            get(profile_routes::get_login).delete(profile_routes::cancel_login),
        )
        .route("/api/profiles/:id/login/:flow_id/code", post(profile_routes::submit_login_code))
        .route("/api/profiles/:id/logout", post(profile_routes::logout))
        .route("/api/engines", get(engine_routes::list))
        .route("/api/engines/recheck", post(engine_routes::recheck))
        .route("/api/engines/claude/login", post(engine_login::start))
        .route(
            "/api/engines/claude/login/:sid",
            get(engine_login::get).delete(engine_login::cancel),
        )
        .route("/api/engines/claude/login/:sid/code", post(engine_login::submit_code))
        .route("/api/engines/claude/logout", post(engine_login::logout))
        .route("/api/engines/:id", post(engine_routes::set_enabled).put(engine_routes::set_enabled))
        .route("/api/laya/status", get(routes::laya_status))
        .route("/api/kwants", get(routes::list_kwants).post(routes::create_kwant))
        .route("/api/kwants/:id", put(routes::put_kwant).delete(routes::delete_kwant))
        .route("/api/kwants/:id/usage", get(routes::kwant_usage))
        .route("/api/kwants/:id/export", get(import_routes::export_kwant))
        .route("/api/export", get(import_routes::export_space))
        .route("/api/import/prompts", get(import_routes::prompts))
        .route("/api/import/preview", post(import_routes::preview))
        .route("/api/import/apply", post(import_routes::apply))
        .route("/api/imports", get(import_routes::list))
        .route("/api/imports/:id", get(import_routes::get))
        .route("/api/imports/:id/undo", post(import_routes::undo))
        .route("/api/channels", get(routes::list_channels).post(routes::create_channel))
        .route("/api/channels/:id/messages", get(routes::list_messages))
        .route("/api/teams", get(routes::list_teams).post(routes::create_team))
        .route(
            "/api/teams/:id",
            get(routes::get_team).put(routes::update_team).patch(continuity::patch_team).delete(routes::delete_team),
        )
        .route("/api/chat/send", post(routes::send_chat))
        .route("/api/chat/cancel", post(routes::cancel_chat))
        .route("/api/approvals", get(approval_modes::list))
        .route("/api/approvals/:id/resolve", post(routes::resolve_approval))
        .route("/api/spaces/:id/approvals", get(approval_modes::get_space).put(approval_modes::put_space))
        .route("/api/kwants/:id/approvals", get(approval_modes::get_kwant).put(approval_modes::put_kwant))
        .route("/api/settings", get(routes::get_settings).post(routes::save_settings))
        .route("/api/settings/computer", get(computer::get_computer).put(computer::put_computer))
        .route("/api/settings/computer/window", put(computer::put_window))
        .route("/api/system/directories", get(routes::list_system_directories))
        .route("/api/system/select-folder", post(routes::select_folder))
        .route("/api/sandbox/:team_id/status", get(routes::get_sandbox_status))
        .route("/api/sandbox/:team_id/screenshot", get(routes::get_browser_screenshot))
        .route("/api/sandbox/:team_id/restart", post(routes::restart_sandbox))
        .route("/api/sandbox/:team_id/navigate", post(routes::navigate_sandbox))
        .route("/api/sandbox/:team_id/interact", post(routes::interact_sandbox))
        .route("/api/sandbox/:team_id/stream", get(browser_stream::upgrade))
        .route("/api/sandbox/:team_id/page", get(routes::get_sandbox_page))
        .route("/api/sandbox/:team_id/act", post(routes::act_sandbox))
        .route("/api/sandbox/:team_id/cite", post(routes::cite_sandbox))
        .route("/api/sandbox/:team_id/passwords", get(passwords::list).post(passwords::save))
        .route("/api/sandbox/:team_id/passwords/pending", post(passwords::save_pending).delete(passwords::discard_pending))
        .route("/api/sandbox/:team_id/passwords/:id", delete(passwords::remove))
        .route("/api/sandbox/:team_id/passwords/:id/reveal", post(passwords::reveal))
        .route("/api/passwords/store", get(passwords::store_status))
        .route("/api/passwords/bitwarden", delete(passwords::forget_bitwarden))
        .route("/api/passwords/bitwarden/unlock", post(passwords::unlock_bitwarden))
        .route("/api/passwords/bitwarden/lock", post(passwords::lock_bitwarden))
        .route("/api/passwords/migrate", post(passwords::migrate_to_bitwarden))
        .route("/api/setup/status", get(setup_routes::status))
        .route("/api/setup/install", post(setup_routes::install))
        .route("/api/setup/jobs/:id", get(setup_routes::job))
        .route("/api/zombie/cases", get(zombie_routes::list_cases))
        .route("/api/zombie/cases/:id", get(zombie_routes::get_case))
        .route("/api/zombie/cases/:id/snooze", post(zombie_routes::snooze))
        .route("/api/zombie/cases/:id/dismiss", post(zombie_routes::dismiss))
        .route("/api/zombie/cases/:id/done", post(zombie_routes::done))
        .route("/api/zombie/cases/:id/followed-up", post(zombie_routes::followed_up))
        .route("/api/zombie/cases/:id/reopen", post(zombie_routes::reopen))
        .route("/api/zombie/cases/:id/draft", post(zombie_routes::regenerate_draft))
        .route("/api/zombie/cases/:id/fields", patch(document_routes::patch_fields))
        .route("/api/documents", post(document_routes::create))
        .route("/api/documents/:id", get(document_routes::get_one).delete(document_routes::delete))
        .route("/api/zombie/sources", get(zombie_routes::list_sources).post(zombie_routes::add_source))
        .route("/api/zombie/sources/:id", patch(zombie_routes::update_source).delete(zombie_routes::delete_source))
        .route("/api/zombie/sources/:id/test", post(zombie_routes::test_source))
        .route("/api/zombie/run", post(zombie_routes::run))
        .route("/api/zombie/status", get(zombie_routes::status))
        .route("/api/zombie/settings", get(zombie_routes::get_settings).put(zombie_routes::put_settings))
        .route("/api/zombie/reviews", get(zombie_routes::reviews))
        .route("/api/zombie/owner", get(zombie_routes::get_owner).put(zombie_routes::put_owner))
        .route("/api/parallel/scenarios", get(parallel_routes::list).post(parallel_routes::create))
        .route(
            "/api/parallel/scenarios/:id",
            get(parallel_routes::get_one).put(parallel_routes::update).delete(parallel_routes::delete),
        )
        .route("/api/parallel/prices", get(parallel_routes::list_prices))
        .route("/api/parallel/estimate", post(parallel_routes::estimate))
        .route("/api/parallel/scenarios/:id/run", post(parallel_routes::start_run))
        .route("/api/parallel/scenarios/:id/runs", get(parallel_routes::scenario_runs))
        .route("/api/parallel/runs", get(parallel_routes::list_runs))
        .route("/api/parallel/runs/:id", get(parallel_routes::get_run))
        .route("/api/connections", get(connections::list))
        .route("/api/connections/composio", post(connections::add_composio))
        .route("/api/connections/composio/apps", get(connections::search_composio_apps))
        .route("/api/connections/composio/apps/:slug/connect", post(connections::connect_composio_app))
        .route("/api/connections/composio/apps/:slug/status", get(connections::composio_app_status))
        .route("/api/mcp/servers", get(mcp_routes::list_servers).post(mcp_routes::create_server))
        .route(
            "/api/mcp/servers/:id",
            get(mcp_routes::get_server).put(mcp_routes::update_server).delete(mcp_routes::delete_server),
        )
        .route("/api/mcp/servers/:id/test", post(mcp_routes::test_server))
        .route("/api/mcp/servers/:id/oauth/start", post(mcp_oauth::start))
        .route("/api/mcp/servers/:id/oauth", delete(mcp_oauth::disconnect))
        .route("/api/mcp/oauth/callback", get(mcp_oauth::callback))
        .route("/api/kwants/:id/mcp-servers", get(mcp_routes::get_kwant_servers).put(mcp_routes::set_kwant_servers))
        .route("/api/mcp/permission/:token", post(mcp_runtime::permission_endpoint))
        .route("/api/mcp/gateway", post(mcp_runtime::gateway_endpoint))
        .route("/api/mcp/gateway/grok", post(mcp_runtime::gateway_grok_endpoint))
        .route("/api/mcp/self", post(continuity::self_endpoint))
        .route("/api/mcp/suggestions", get(mcp_suggest::list))
        .route("/api/mcp/suggestions/:id/install", post(mcp_suggest::install))
        .route("/api/mcp/suggestions/:id/dismiss", post(mcp_suggest::dismiss))
        .route("/api/bot-actions", get(bot_actions::list))
        .route("/api/bot-actions/:id/undo", post(bot_actions::undo))
        .route("/api/bot-actions/:id/confirm", post(bot_actions::confirm))
        .route("/api/bot-actions/:id/decline", post(bot_actions::decline))
        .route("/api/route-choices/:id/resolve", post(rerouting::resolve_route))
        .route("/api/hires", get(hires::list))
        .route("/api/hires/:id/accept", post(hires::accept_route))
        .route("/api/hires/:id/deny", post(hires::deny_route))
        .route("/api/spaces/:id/hire-policy", get(hires::get_policy).put(hires::put_policy))
        .route("/api/channels/:id", get(continuity::get_channel).patch(continuity::patch_channel))
        .route("/api/search", get(continuity::search))
        .route("/api/kwants/:id/memories", get(continuity::list_memories).post(continuity::add_memory))
        .route("/api/kwants/:id/model-usage", get(model_usage::get_usage))
        .route("/api/usage/daily", get(model_usage::get_daily))
        .route("/api/chains/:id/continue", post(budget::continue_chain))
        .route(
            "/api/kwants/:id/memories/:mid",
            axum::routing::patch(continuity::update_memory).delete(continuity::delete_memory),
        )
        .route("/api/routines", get(scheduler::list).post(scheduler::create))
        .route(
            "/api/routines/:id",
            get(scheduler::get).patch(scheduler::update).delete(scheduler::remove),
        )
        .route("/api/routines/:id/run", post(scheduler::run_now))
        .route("/api/routines/:id/runs", get(scheduler::runs))
        .route("/api/turns", get(delegation::list_turns))
        .route("/api/kwants/:id/threads", get(delegation::list_threads).post(delegation::create_thread))
        .route("/api/threads/:id", get(delegation::get_thread).patch(delegation::patch_thread))
        .route("/api/threads/:id/messages", get(delegation::thread_messages))
        .route("/api/threads/:id/cancel", post(delegation::cancel_thread))
        .route("/api/delegations", get(delegation::list_delegations).post(delegation::create_delegation))
        .route("/api/delegations/:id", get(delegation::get_delegation))
        .route("/api/delegations/:id/cancel", post(delegation::cancel_delegation))
        .route("/api/attachments", get(attachments::list).post(attachments::upload))
        .route("/api/attachments/:id", get(attachments::get))
        .route("/api/attachments/:id/content", get(attachments::content))
        .route("/api/waiting", get(waiting::list))
        .route("/api/questions", get(questions::list))
        .route("/api/questions/:id", get(questions::get))
        .route("/api/questions/:id/answer", post(questions::answer))
        .route("/api/messages/:message_id/decisions/:index/change", post(decide::change))
        .route("/api/work-ahead/run", post(work_ahead::run_now))
        .route("/api/missions", get(missions::routes::list_missions).post(missions::routes::create_mission))
        .route("/api/missions/insights", get(missions::routes::insights))
        .route("/api/performance", get(missions::performance::report))
        .route("/api/missions/:id", get(missions::routes::get_mission).patch(missions::routes::patch_mission))
        .route("/api/missions/:id/approve", post(missions::routes::approve_mission))
        .route("/api/missions/:id/cancel", post(missions::routes::cancel_mission))
        .route("/api/missions/:id/run", post(missions::routes::run_mission))
        .route("/api/missions/:id/graph", get(missions::graph::mission_graph))
        .route("/api/missions/:id/graph/scan", post(missions::graph::scan))
        .route("/api/graph", get(missions::graph::loose_graph))
        .route("/api/tasks", get(missions::routes::list_tasks).post(missions::routes::create_task))
        .route("/api/tasks/:id", get(missions::routes::get_task).patch(missions::routes::patch_task))
        .route("/api/tasks/:id/verify", post(missions::routes::verify_task))
        .route("/api/tasks/:id/reject", post(missions::routes::reject_task))
        .route("/api/tasks/:id/block", post(missions::routes::block_task))
        .route("/api/tasks/:id/unblock", post(missions::routes::unblock_task))
        .route("/api/tasks/:id/retry", post(missions::routes::retry_task))
        .route("/api/tasks/:id/cancel", post(missions::routes::cancel_task))
        .route("/api/tasks/:id/fork", post(missions::graph::fork_task))
        .route("/api/tasks/:id/run", post(missions::checkpoints::run_held))
        .route("/api/tasks/:id/checkpoints", get(missions::checkpoints::list).post(missions::checkpoints::create))
        .route("/api/tasks/:id/checkpoints/:cid/restore-preview", get(missions::checkpoints::preview))
        .route("/api/tasks/:id/checkpoints/:cid/restore", post(missions::checkpoints::restore))
        .route("/api/tasks/:id/flags", get(missions::graph::task_flags))
        .route("/api/tasks/:id/risks", get(missions::auditor::task_risks))
        .route("/api/tasks/:id/risks/:risk_id/resolve", post(missions::auditor::resolve_risk))
        .route("/api/tasks/:id/audit", post(missions::auditor::start_audit))
        .route("/api/risks", get(missions::auditor::space_risks))
        .route("/api/tasks/:id/failures", get(missions::autofix::task_failures))
        .route("/api/tasks/:id/autofix", post(missions::autofix::run_autofix))
        .route("/api/tasks/:id/autofix/:aid/cancel", post(missions::autofix::cancel_autofix))
        .route("/api/workflow-changes", get(missions::autofix::list_changes))
        .route("/api/capabilities", get(missions::flight_school::get_capabilities).post(missions::flight_school::set_capability))
        .route("/api/capabilities/:kwant_id/revoke", post(missions::flight_school::revoke_capability))
        .route("/api/capabilities/:kwant_id/scorecard", get(missions::flight_school::get_scorecard))
        .route("/api/capabilities/proposals", get(missions::flight_school::list_proposals))
        .route("/api/capabilities/proposals/:id/grant", post(missions::flight_school::grant_proposal))
        .route("/api/capabilities/proposals/:id/decline", post(missions::flight_school::decline_proposal))
        .route("/api/workflow-changes/:id/approve", post(missions::autofix::approve_change))
        .route("/api/workflow-changes/:id/reject", post(missions::autofix::reject_change))
        .route("/api/tasks/:id/flags/:flag_id/resolve", post(missions::graph::resolve_flag))
        .route("/api/tasks/:id/checks", post(missions::routes::add_task_check))
        .route("/api/tasks/:id/checks/run", post(missions::routes::run_task_checks))
        .route("/api/checks/:id/approve", post(missions::routes::approve_check))
        .route("/api/questions/:id/cancel", post(questions::cancel))
        .route("/api/human-steps", get(human_steps::list))
        .route("/api/human-steps/:id", get(human_steps::get))
        .route("/api/human-steps/:id/done", post(human_steps::done))
        .route("/api/human-steps/:id/decline", post(human_steps::decline))
        .route("/api/automation-discovery", get(missions::discovery::list))
        .route("/api/automation-discovery/scan", post(missions::discovery::scan))
        .route("/api/automation-discovery/:id/accept", post(missions::discovery::accept))
        .route("/api/automation-discovery/:id/dismiss", post(missions::discovery::dismiss))
        .route("/api/facts", get(missions::truth_ledger::list).post(missions::truth_ledger::create))
        .route("/api/facts/disputes", get(missions::truth_ledger::disputes))
        .route("/api/facts/disputes/:id/resolve", post(missions::truth_ledger::resolve))
        .route("/api/facts/disputes/:id/dismiss", post(missions::truth_ledger::dismiss))
        .route("/api/facts/:id", get(missions::truth_ledger::get))
        .route("/api/facts/:id/promote", post(missions::truth_ledger::promote))
        .route("/api/facts/:id/expire", post(missions::truth_ledger::expire))
        .route("/api/facts/:id/evidence", post(missions::truth_ledger::add_evidence))
        .route("/api/facts/:id/uncite", post(missions::truth_ledger::uncite))
        .route("/api/facts/:id/accept", post(missions::truth_ledger::accept))
        .route("/api/facts/:id/reject", post(missions::truth_ledger::reject))
        .route("/api/skills/foundry", get(missions::foundry::overview))
        .route("/api/skills/foundry/:cluster/draft", post(missions::foundry::draft_now))
        .route("/api/skills/:id/foundry", get(missions::foundry::detail))
        .route("/api/skills/:id/replay", post(missions::foundry::replay))
        .route("/api/skills/:id/rollback", post(missions::foundry::rollback))
        .route("/api/skills", get(skills::list).post(skills::create))
        .route("/api/skills/:id", get(skills::get).patch(skills::update).delete(skills::remove))
        .route("/api/skills/:id/diff", get(skills::diff))
        .route("/api/skills/:id/approve", post(skills::approve))
        .route("/api/skills/:id/reject", post(skills::reject))
        .route("/api/away", get(away::get))
        .route("/api/away/seen", post(away::seen))
        .route("/api/events", get(sse::sse_handler))
        .fallback(crate::embedded::static_handler)
        .layer(axum::middleware::from_fn_with_state(state.clone(), spaces::row_guard))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth::guard))
        .with_state(state)
}
