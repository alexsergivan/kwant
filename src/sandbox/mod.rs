pub mod bitwarden;
pub mod browser;
pub mod computer;
pub mod credentials;
pub mod manager;
pub mod service;
pub mod vault;

pub use browser::{detect_bot_wall, BrowserController, BrowserError, NavigationResult, PendingCapture};
pub use credentials::{Credential, CredentialMeta, CredentialStore, EncryptedFileStore};
pub use manager::{ActiveSandbox, SandboxError, SandboxManager, ShellOutput};

