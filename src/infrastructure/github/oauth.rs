use super::AccessToken;
use crate::infrastructure::InfrastructureError;
use std::{fmt, time::Duration};
use zeroize::Zeroizing;

pub struct DeviceAuthorization {
    device_code: Zeroizing<String>,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: Duration,
    pub expires_in: Duration,
}

impl DeviceAuthorization {
    pub(crate) fn new(
        device_code: String,
        user_code: String,
        verification_uri: String,
        interval_seconds: u64,
        expires_in_seconds: u64,
    ) -> Self {
        Self {
            device_code: Zeroizing::new(device_code),
            user_code,
            verification_uri,
            interval: Duration::from_secs(interval_seconds.max(1)),
            expires_in: Duration::from_secs(expires_in_seconds),
        }
    }

    pub(crate) fn device_code(&self) -> &str {
        &self.device_code
    }
}

impl fmt::Debug for DeviceAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceAuthorization")
            .field("device_code", &"[REDACTED]")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("interval", &self.interval)
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

#[derive(Debug)]
pub enum PollResult {
    Pending,
    SlowDown(Duration),
    Token(AccessToken),
}

pub fn ensure_poll_allowed(
    elapsed: Duration,
    expires_in: Duration,
    cancelled: bool,
) -> Result<(), InfrastructureError> {
    if cancelled {
        Err(InfrastructureError::Cancelled)
    } else if elapsed >= expires_in {
        Err(InfrastructureError::OAuthExpired)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_and_expiry_are_terminal() {
        assert!(matches!(
            ensure_poll_allowed(Duration::ZERO, Duration::from_secs(10), true),
            Err(InfrastructureError::Cancelled)
        ));
        assert!(matches!(
            ensure_poll_allowed(Duration::from_secs(10), Duration::from_secs(10), false),
            Err(InfrastructureError::OAuthExpired)
        ));
    }

    #[test]
    fn debug_never_contains_device_secret() {
        let authorization = DeviceAuthorization::new(
            "device-secret".to_owned(),
            "ABCD-1234".to_owned(),
            "https://github.com/login/device".to_owned(),
            5,
            900,
        );
        assert!(!format!("{authorization:?}").contains("device-secret"));
    }
}
