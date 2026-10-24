pub mod broker;
pub mod config;
pub mod db;
pub mod doctor;
pub mod documents;
pub mod drivers;
pub mod embedded;
pub mod import;
pub mod mcp;
pub mod parallel;
pub mod profiles;
pub mod router;
pub mod routines;
pub mod sandbox;
pub mod serve;
pub mod server;
pub mod tools;
pub mod zombie;

pub use broker::{ApprovalDecision, ApprovalRequest, PermissionBroker};
pub use config::{Config, ConfigError};
pub use db::{Database, DbError};
pub use drivers::{create_driver_for_kwant, ApiDriver, CliDriver, Driver, DriverError, RuntimeEvent, TurnInput, TurnOutput};
pub use embedded::{static_handler, Assets};
pub use router::{
    parse_mentions, LayaRouter, RouteDecision, RouterError, TagMention, TeamDecision,
};
pub use sandbox::{
    ActiveSandbox, BrowserController, BrowserError, NavigationResult, SandboxError, SandboxManager,
    ShellOutput,
};
pub use server::{create_router, AppState};
pub use tools::{ToolError, ToolRunner};

/// Makes kwnt's own process non-dumpable, so another process of the same OS user (a bot's shell
/// with approvals off) can't read its memory through `/proc/<pid>/mem`, attach a debugger, or
/// read `/proc/<pid>/environ`: those files become root's. kwnt holds the unlocked Bitwarden
/// keys in memory only (see `sandbox::bitwarden`). Children kwnt starts get the default back at
/// `execve`, so nothing they do changes. Linux only; elsewhere a no-op.
pub fn harden_process() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: prctl(PR_SET_DUMPABLE) takes plain integers and touches no memory of ours
        let rc = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
        if rc != 0 {
            eprintln!("kwnt: could not make the process non-dumpable: {}", std::io::Error::last_os_error());
        }
    }
}
