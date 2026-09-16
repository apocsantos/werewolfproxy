use crate::fang_registry::FangRegistry;
use crate::target_policy::TargetPolicy;
use werewolf_core::{
    fang_profile::FangProfile, pack::PeerRecord, pelt::PeltIdentity, state::Status,
};

/// A persistent readiness condition, not a one-shot event. The Pelt and its
/// runtime certificate stay in DaemonState; only their availability is watched.
pub(super) struct TlsIdentityReady(tokio::sync::watch::Sender<bool>);

impl Default for TlsIdentityReady {
    fn default() -> Self {
        let (sender, _) = tokio::sync::watch::channel(false);
        Self(sender)
    }
}

impl TlsIdentityReady {
    pub(super) fn subscribe(&self) -> tokio::sync::watch::Receiver<bool> {
        self.0.subscribe()
    }

    pub(super) fn publish(&self) {
        // send_replace retains READY even if no listener has subscribed yet.
        self.0.send_replace(true);
    }

    #[cfg(test)]
    pub(super) fn is_ready(&self) -> bool {
        *self.0.borrow()
    }
}

/// Subscribe before checking state. A publication between that check and
/// wait_for is retained by watch and therefore cannot strand a listener.
pub(super) async fn wait_for_runtime_tls_identity(
    state: &std::sync::Arc<tokio::sync::Mutex<DaemonState>>,
) -> std::io::Result<std::sync::Arc<crate::tls_identity::RuntimeTlsIdentity>> {
    let mut ready = state.lock().await.tls_identity_ready.subscribe();
    loop {
        let live = state.lock().await;
        if let Some(identity) = live.runtime_tls_identity.clone() {
            return Ok(identity);
        }
        if *ready.borrow() {
            return Err(std::io::Error::other(
                "runtime TLS identity ready without published identity",
            ));
        }
        #[cfg(test)]
        if let Some(observed) = &live.tls_identity_wait_observed {
            let _ = observed.send(());
        }
        #[cfg(test)]
        let wait_gate = live.tls_identity_wait_gate.clone();
        drop(live);
        #[cfg(test)]
        if let Some(gate) = wait_gate {
            let _permit = gate.acquire().await.map_err(|_| {
                std::io::Error::other("test identity wait gate unexpectedly closed")
            })?;
        }
        ready
            .wait_for(|available| *available)
            .await
            .map_err(|_| std::io::Error::other("runtime TLS identity readiness closed"))?;
    }
}

#[derive(Default)]
pub(super) struct DaemonState {
    pub(super) inbound_authority: std::sync::Arc<crate::authority::Authority>,
    pub(super) storage_degraded: bool,
    pub(super) den: Option<std::sync::Arc<werewolf_core::local_fs::PrivateDirectory>>,
    pub(super) peers: Vec<PeerRecord>,
    pub(super) fang_registry: FangRegistry,
    pub(super) fang_profiles: Vec<FangProfile>,
    pub(super) active_profiles: Vec<String>,
    /// Present only after explicit Stage15B migration. This is local
    /// consistency metadata and is never sent over a network transport.
    pub(super) protected_state_generation: Option<u64>,
    pub(super) admission: std::sync::Arc<crate::admission::Admission>,
    pub(super) status: Status,
    pub(super) pelt: Option<PeltIdentity>,
    pub(super) runtime_tls_identity:
        Option<std::sync::Arc<crate::tls_identity::RuntimeTlsIdentity>>,
    pub(super) tls_identity_ready: TlsIdentityReady,
    #[cfg(test)]
    pub(super) tls_identity_wait_observed: Option<tokio::sync::mpsc::UnboundedSender<()>>,
    #[cfg(test)]
    pub(super) tls_identity_wait_gate: Option<std::sync::Arc<tokio::sync::Semaphore>>,
    pub(super) den_socket: String,
    pub(super) den_home: String,
    pub(super) den_listen: String,
    pub(super) den_quic_listen: String,
    pub(super) target_policy: TargetPolicy,
}

impl DaemonState {
    pub(super) fn initialize_runtime_tls_identity(
        &mut self,
    ) -> Result<(), crate::tls_identity::TlsIdentityError> {
        self.runtime_tls_identity = self
            .pelt
            .as_ref()
            .map(crate::tls_identity::RuntimeTlsIdentity::from_pelt)
            .transpose()?
            .map(std::sync::Arc::new);
        if self.runtime_tls_identity.is_some() {
            self.tls_identity_ready.publish();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};
    use tokio::{sync::Mutex, time::timeout};
    use werewolf_core::pelt::generate_identity;

    #[tokio::test]
    async fn identity_publication_between_absent_check_and_wait_is_retained() {
        let (observed, mut waits) = tokio::sync::mpsc::unbounded_channel();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let state = Arc::new(Mutex::new(DaemonState {
            tls_identity_wait_observed: Some(observed),
            tls_identity_wait_gate: Some(gate.clone()),
            ..Default::default()
        }));
        let waiting_state = state.clone();
        let listener =
            tokio::spawn(async move { wait_for_runtime_tls_identity(&waiting_state).await });
        timeout(Duration::from_secs(1), waits.recv())
            .await
            .unwrap()
            .unwrap();
        // This is the exact gap in the former Notify loop: publication occurs
        // after the production helper's absent check but before its watch wait.
        let pelt = generate_identity();
        let identity = Arc::new(crate::tls_identity::RuntimeTlsIdentity::from_pelt(&pelt).unwrap());
        {
            let mut live = state.lock().await;
            live.pelt = Some(pelt);
            live.runtime_tls_identity = Some(identity.clone());
            live.tls_identity_ready.publish();
        }
        gate.add_permits(1);
        let found = timeout(Duration::from_secs(1), listener)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&found, &identity));
    }

    #[tokio::test]
    async fn readiness_persists_for_multiple_early_and_late_listeners() {
        let state = Arc::new(Mutex::new(DaemonState::default()));
        let mut early = {
            let live = state.lock().await;
            [
                live.tls_identity_ready.subscribe(),
                live.tls_identity_ready.subscribe(),
            ]
        };
        let pelt = generate_identity();
        let identity = Arc::new(crate::tls_identity::RuntimeTlsIdentity::from_pelt(&pelt).unwrap());
        {
            let mut live = state.lock().await;
            live.pelt = Some(pelt);
            live.runtime_tls_identity = Some(identity.clone());
            live.tls_identity_ready.publish();
        }
        for receiver in &mut early {
            timeout(Duration::from_secs(1), receiver.wait_for(|value| *value))
                .await
                .unwrap()
                .unwrap();
        }
        for _ in 0..2 {
            let found = timeout(
                Duration::from_secs(1),
                wait_for_runtime_tls_identity(&state),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(Arc::ptr_eq(&found, &identity));
        }
    }

    #[tokio::test]
    async fn identity_ready_without_identity_fails_closed() {
        let state = Arc::new(Mutex::new(DaemonState::default()));
        state.lock().await.tls_identity_ready.publish();
        assert!(timeout(
            Duration::from_secs(1),
            wait_for_runtime_tls_identity(&state)
        )
        .await
        .unwrap()
        .is_err());
    }

    #[tokio::test]
    async fn startup_readiness_matches_existing_pelt() {
        let mut fresh = DaemonState::default();
        fresh.initialize_runtime_tls_identity().unwrap();
        assert!(fresh.runtime_tls_identity.is_none());
        assert!(!fresh.tls_identity_ready.is_ready());

        let pelt = generate_identity();
        let mut restored = DaemonState {
            pelt: Some(pelt.clone()),
            ..Default::default()
        };
        restored.initialize_runtime_tls_identity().unwrap();
        assert!(restored.tls_identity_ready.is_ready());
        let identity = restored.runtime_tls_identity.as_ref().unwrap();
        let parsed = webpki::EndEntityCert::try_from(identity.certificate()).unwrap();
        let canonical =
            crate::tls_identity::canonical_spki_from_public_key_b64(&pelt.public_key_b64).unwrap();
        assert_eq!(parsed.subject_public_key_info().as_ref(), canonical);
    }

    #[tokio::test]
    async fn publishing_identity_does_not_unlock_silver() {
        let pelt = generate_identity();
        let authority = crate::authority::Authority::new(true);
        let mut state = DaemonState {
            inbound_authority: authority.clone(),
            ..Default::default()
        };
        let identity = Arc::new(crate::tls_identity::RuntimeTlsIdentity::from_pelt(&pelt).unwrap());
        state.pelt = Some(pelt);
        state.runtime_tls_identity = Some(identity);
        state.tls_identity_ready.publish();
        assert!(state.tls_identity_ready.is_ready());
        assert!(authority.is_locked());
        assert!(state.inbound_authority.is_locked());
    }
}
