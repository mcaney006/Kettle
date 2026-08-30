use super::{AppState, AuthState, OperationState, RefreshStage};
use crate::domain::BrewAction;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct AppController {
    pub state: AppState,
    refresh_generation: AtomicU64,
    refresh_cancel: Option<CancellationToken>,
    auth_cancel: Option<CancellationToken>,
    mutation_cancel: Option<CancellationToken>,
}

impl Default for AppController {
    fn default() -> Self {
        Self {
            state: AppState::default(),
            refresh_generation: AtomicU64::new(0),
            refresh_cancel: None,
            auth_cancel: None,
            mutation_cancel: None,
        }
    }
}

impl AppController {
    pub fn begin_refresh(&mut self) -> (u64, CancellationToken) {
        if let Some(token) = self.refresh_cancel.take() {
            token.cancel();
        }
        let generation = self.refresh_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let token = CancellationToken::default();
        self.refresh_cancel = Some(token.clone());
        self.state.operation = OperationState::Refreshing(RefreshStage::InstalledState);
        (generation, token)
    }

    pub fn refresh_is_current(&self, generation: u64) -> bool {
        generation == self.refresh_generation.load(Ordering::Acquire)
            && self
                .refresh_cancel
                .as_ref()
                .is_some_and(|token| !token.is_cancelled())
    }

    pub fn finish_refresh(&mut self, generation: u64) {
        if self.refresh_is_current(generation) {
            self.refresh_cancel = None;
            self.state.operation = OperationState::Idle;
        }
    }

    pub fn begin_mutation(
        &mut self,
        action: BrewAction,
        targets: usize,
    ) -> Option<CancellationToken> {
        if self.state.operation != OperationState::Idle || targets == 0 {
            return None;
        }
        let token = CancellationToken::default();
        self.mutation_cancel = Some(token.clone());
        self.state.operation = OperationState::Mutating { action, targets };
        Some(token)
    }

    pub fn finish_mutation(&mut self) {
        self.mutation_cancel = None;
        self.state.operation = OperationState::Idle;
    }

    pub fn begin_authentication(&mut self) -> CancellationToken {
        self.cancel_authentication();
        let token = CancellationToken::default();
        self.auth_cancel = Some(token.clone());
        self.state.auth = AuthState::RequestingDeviceCode;
        token
    }

    pub fn cancel_authentication(&mut self) {
        if let Some(token) = self.auth_cancel.take() {
            token.cancel();
        }
        if matches!(
            self.state.auth,
            AuthState::RequestingDeviceCode | AuthState::AwaitingApproval(_)
        ) {
            self.state.auth = AuthState::SignedOut;
        }
    }

    pub fn finish_authentication(&mut self) {
        self.auth_cancel = None;
    }
}

impl Drop for AppController {
    fn drop(&mut self) {
        if let Some(token) = &self.refresh_cancel {
            token.cancel();
        }
        if let Some(token) = &self.auth_cancel {
            token.cancel();
        }
        if let Some(token) = &self.mutation_cancel {
            token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superseding_refresh_cancels_old_generation() {
        let mut controller = AppController::default();
        let (first, first_token) = controller.begin_refresh();
        let (second, _) = controller.begin_refresh();
        assert!(first_token.is_cancelled());
        assert!(!controller.refresh_is_current(first));
        assert!(controller.refresh_is_current(second));
    }

    #[test]
    fn authentication_is_cancellable() {
        let mut controller = AppController::default();
        let token = controller.begin_authentication();
        controller.cancel_authentication();
        assert!(token.is_cancelled());
        assert_eq!(controller.state.auth, AuthState::SignedOut);
    }
}
