use crate::fang_registry::FangRegistry;
use crate::target_policy::TargetPolicy;
use werewolf_core::{
    fang_profile::FangProfile, pack::PeerRecord, pelt::PeltIdentity, state::Status,
};

#[derive(Default)]
pub(super) struct DaemonState {
    pub(super) peers: Vec<PeerRecord>,
    pub(super) fang_registry: FangRegistry,
    pub(super) fang_profiles: Vec<FangProfile>,
    pub(super) admission: std::sync::Arc<crate::admission::Admission>,
    pub(super) status: Status,
    pub(super) pelt: Option<PeltIdentity>,
    pub(super) den_socket: String,
    pub(super) den_home: String,
    pub(super) den_listen: String,
    pub(super) den_quic_listen: String,
    pub(super) target_policy: TargetPolicy,
}
