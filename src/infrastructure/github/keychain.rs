use super::AccessToken;
use crate::infrastructure::InfrastructureError;
use zeroize::Zeroizing;

const SERVICE: &str = "com.kettle.app.github";
const ACCOUNT: &str = "oauth-token";

pub trait TokenStore: Send + Sync {
    fn store(&self, token: &AccessToken) -> Result<(), InfrastructureError>;
    fn load(&self) -> Result<Option<AccessToken>, InfrastructureError>;
    fn delete(&self) -> Result<(), InfrastructureError>;
}

#[derive(Default)]
pub struct MacKeychain;

impl TokenStore for MacKeychain {
    fn store(&self, token: &AccessToken) -> Result<(), InfrastructureError> {
        security_framework::passwords::set_generic_password(
            SERVICE,
            ACCOUNT,
            token.expose().as_bytes(),
        )
        .map_err(|error| InfrastructureError::Keychain(error.to_string()))
    }

    fn load(&self) -> Result<Option<AccessToken>, InfrastructureError> {
        match security_framework::passwords::get_generic_password(SERVICE, ACCOUNT) {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                let token = std::str::from_utf8(&bytes)
                    .map_err(|_| InfrastructureError::InvalidUtf8("Keychain OAuth token"))?;
                Ok(Some(AccessToken::new(token.to_owned())))
            }
            Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => {
                Ok(None)
            }
            Err(error) => Err(InfrastructureError::Keychain(error.to_string())),
        }
    }

    fn delete(&self) -> Result<(), InfrastructureError> {
        match security_framework::passwords::delete_generic_password(SERVICE, ACCOUNT) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => {
                Ok(())
            }
            Err(error) => Err(InfrastructureError::Keychain(error.to_string())),
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct FakeTokenStore(Mutex<Option<String>>);

    impl TokenStore for FakeTokenStore {
        fn store(&self, token: &AccessToken) -> Result<(), InfrastructureError> {
            *self.0.lock().unwrap() = Some(token.expose().to_owned());
            Ok(())
        }

        fn load(&self) -> Result<Option<AccessToken>, InfrastructureError> {
            Ok(self.0.lock().unwrap().clone().map(AccessToken::new))
        }

        fn delete(&self) -> Result<(), InfrastructureError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[test]
    fn tests_use_a_fake_not_the_real_keychain() {
        let store = FakeTokenStore::default();
        store.store(&AccessToken::new("secret".to_owned())).unwrap();
        assert!(store.load().unwrap().is_some());
        store.delete().unwrap();
        assert!(store.load().unwrap().is_none());
    }
}
