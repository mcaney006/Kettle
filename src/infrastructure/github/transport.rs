use super::{CLIENT_ID, DeviceAuthorization, PollResult, SCOPE};
use crate::infrastructure::InfrastructureError;
use reqwest::{StatusCode, blocking::Client};
use serde::Deserialize;
use std::{fmt, time::Duration};
use zeroize::Zeroizing;

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const USER_URL: &str = "https://api.github.com/user";

pub struct AccessToken(Zeroizing<String>);

impl AccessToken {
    pub fn new(secret: String) -> Self {
        Self(Zeroizing::new(secret))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccessToken([REDACTED])")
    }
}

pub trait OAuthTransport: Send + Sync {
    fn request_device_code(&self) -> Result<DeviceAuthorization, InfrastructureError>;
    fn poll(&self, authorization: &DeviceAuthorization) -> Result<PollResult, InfrastructureError>;
    fn whoami(&self, token: &AccessToken) -> Result<String, InfrastructureError>;
}

pub struct GitHubTransport {
    client: Client,
}

impl GitHubTransport {
    pub fn new() -> Result<Self, InfrastructureError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .user_agent(concat!("Kettle/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(classify_transport_error)?;
        Ok(Self { client })
    }

    fn post_form<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        form: &[(&str, &str)],
        context: &'static str,
    ) -> Result<T, InfrastructureError> {
        let response = self
            .client
            .post(url)
            .header("Accept", "application/json")
            .form(form)
            .send()
            .map_err(classify_transport_error)?;
        parse_response(
            response.status(),
            response.text().map_err(classify_transport_error)?,
            context,
        )
    }
}

impl OAuthTransport for GitHubTransport {
    fn request_device_code(&self) -> Result<DeviceAuthorization, InfrastructureError> {
        let response: DeviceResponse = self.post_form(
            DEVICE_CODE_URL,
            &[("client_id", CLIENT_ID), ("scope", SCOPE)],
            "GitHub device authorization",
        )?;
        Ok(DeviceAuthorization::new(
            response.device_code,
            response.user_code,
            response.verification_uri,
            response.interval,
            response.expires_in,
        ))
    }

    fn poll(&self, authorization: &DeviceAuthorization) -> Result<PollResult, InfrastructureError> {
        let response: TokenResponse = self.post_form(
            ACCESS_TOKEN_URL,
            &[
                ("client_id", CLIENT_ID),
                ("device_code", authorization.device_code()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ],
            "GitHub access token",
        )?;
        if let Some(token) = response.access_token {
            return Ok(PollResult::Token(AccessToken::new(token)));
        }
        let description = response
            .error_description
            .unwrap_or_else(|| "GitHub did not provide a description".to_owned());
        match response.error.as_deref() {
            Some("authorization_pending") => Ok(PollResult::Pending),
            Some("slow_down") => Ok(PollResult::SlowDown(Duration::from_secs(5))),
            Some("access_denied") => Err(InfrastructureError::OAuthDenied(description)),
            Some("expired_token") => Err(InfrastructureError::OAuthExpired),
            Some(code) => Err(InfrastructureError::OAuthProtocol(format!(
                "{code}: {description}"
            ))),
            None => Err(InfrastructureError::OAuthProtocol(
                "GitHub returned neither a token nor an OAuth error".to_owned(),
            )),
        }
    }

    fn whoami(&self, token: &AccessToken) -> Result<String, InfrastructureError> {
        let response = self
            .client
            .get(USER_URL)
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(token.expose())
            .send()
            .map_err(classify_transport_error)?;
        let user: UserResponse = parse_response(
            response.status(),
            response.text().map_err(classify_transport_error)?,
            "GitHub user profile",
        )?;
        Ok(user.login)
    }
}

fn classify_transport_error(error: reqwest::Error) -> InfrastructureError {
    if error.is_timeout() {
        InfrastructureError::NetworkTimeout
    } else {
        InfrastructureError::NetworkTransport(error)
    }
}

fn parse_response<T: for<'de> Deserialize<'de>>(
    status: StatusCode,
    body: String,
    context: &'static str,
) -> Result<T, InfrastructureError> {
    if !status.is_success() {
        let description = serde_json::from_str::<ErrorResponse>(&body)
            .ok()
            .and_then(|error| error.error_description.or(error.error).or(error.message))
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        return Err(InfrastructureError::OAuthProtocol(description));
    }
    serde_json::from_str(&body).map_err(|source| InfrastructureError::Json { context, source })
}

#[derive(Deserialize)]
struct DeviceResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
    expires_in: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct UserResponse {
    login: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: Option<String>,
    error_description: Option<String>,
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TimeoutTransport;

    impl OAuthTransport for TimeoutTransport {
        fn request_device_code(&self) -> Result<DeviceAuthorization, InfrastructureError> {
            Err(InfrastructureError::NetworkTimeout)
        }

        fn poll(&self, _: &DeviceAuthorization) -> Result<PollResult, InfrastructureError> {
            Err(InfrastructureError::NetworkTimeout)
        }

        fn whoami(&self, _: &AccessToken) -> Result<String, InfrastructureError> {
            Err(InfrastructureError::NetworkTimeout)
        }
    }

    #[test]
    fn scope_is_profile_only() {
        assert_eq!(SCOPE, "read:user");
        assert!(!SCOPE.contains("repo"));
        assert!(!SCOPE.contains("write"));
    }

    #[test]
    fn oauth_errors_keep_the_server_description() {
        let result = parse_response::<TokenResponse>(
            StatusCode::BAD_REQUEST,
            r#"{"error":"bad_verification_code","error_description":"Code was invalid"}"#
                .to_owned(),
            "test",
        );
        let Err(error) = result else {
            panic!("expected OAuth error response");
        };
        assert!(error.to_string().contains("Code was invalid"));
    }

    #[test]
    fn token_debug_is_redacted() {
        let token = AccessToken::new("bearer-secret".to_owned());
        assert!(!format!("{token:?}").contains("bearer-secret"));
    }

    #[test]
    fn network_timeout_stays_distinct_from_protocol_failure() {
        assert!(matches!(
            TimeoutTransport.request_device_code(),
            Err(InfrastructureError::NetworkTimeout)
        ));
    }
}
