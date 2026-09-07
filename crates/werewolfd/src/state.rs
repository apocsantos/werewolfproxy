use std::{collections::HashMap, time::Instant};
use tokio::task::JoinHandle;
use werewolf_core::{
    fang::FangRecord, fang_profile::FangProfile, pack::PeerRecord, pelt::PeltIdentity,
    state::Status,
};

#[derive(Default)]
pub(super) struct DaemonState {
    pub(super) peers: Vec<PeerRecord>,
    pub(super) fangs: Vec<FangRecord>,
    pub(super) fang_profiles: Vec<FangProfile>,
    pub(super) fang_tasks: HashMap<String, JoinHandle<()>>,
    pub(super) fang_started: HashMap<String, std::time::Instant>,
    pub(super) seen_nonces: HashMap<String, Instant>,
    pub(super) status: Status,
    pub(super) pelt: Option<PeltIdentity>,
    pub(super) den_socket: String,
    pub(super) den_home: String,
    pub(super) den_listen: String,
    pub(super) den_quic_listen: String,
}
