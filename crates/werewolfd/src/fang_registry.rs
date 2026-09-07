use std::{collections::HashMap, time::Instant};
use tokio::task::JoinHandle;
use werewolf_core::fang::FangRecord;

#[derive(Default)]
pub(super) struct FangRegistry {
    pub(super) fangs: Vec<FangRecord>,
    pub(super) tasks: HashMap<String, JoinHandle<()>>,
    pub(super) started: HashMap<String, Instant>,
}
