mod keychain;
mod oauth;
mod transport;

pub use keychain::{MacKeychain, TokenStore};
pub use oauth::{DeviceAuthorization, PollResult, ensure_poll_allowed, validate_verification_uri};
pub use transport::{AccessToken, GitHubTransport, OAuthTransport};

pub const CLIENT_ID: &str = "Ov23liiLF3n6sxqOh6O9";
pub const SCOPE: &str = "read:user";
