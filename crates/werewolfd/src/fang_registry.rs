use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::task::{AbortHandle, JoinHandle};
use werewolf_core::fang::FangRecord;

#[derive(Clone, Default)]
pub(super) struct FangCancellation {
    children: Arc<Mutex<Vec<AbortHandle>>>,
}

impl FangCancellation {
    pub(super) fn track(&self, handle: &JoinHandle<()>) {
        self.children.lock().unwrap().push(handle.abort_handle());
    }

    pub(super) fn abort_children(&self) {
        for handle in self.children.lock().unwrap().drain(..) {
            handle.abort();
        }
    }
}

#[derive(Default)]
pub(super) struct FangRegistry {
    pub(super) fangs: Vec<FangRecord>,
    pub(super) tasks: HashMap<String, JoinHandle<()>>,
    pub(super) started: HashMap<String, Instant>,
    cancellations: HashMap<String, FangCancellation>,
}

impl FangRegistry {
    pub(super) fn len(&self) -> usize {
        self.fangs.len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.fangs.is_empty()
    }
    pub(super) fn records(&self) -> &[FangRecord] {
        &self.fangs
    }
    pub(super) fn has_active_local(&self, local: &str) -> bool {
        self.fangs
            .iter()
            .any(|f| f.local == local && matches!(f.state, werewolf_core::fang::FangState::Active))
    }
    pub(super) fn push(&mut self, fang: FangRecord) {
        self.fangs.push(fang)
    }
    pub(super) fn find_id(&self, id: &str) -> Option<&FangRecord> {
        self.fangs.iter().find(|f| f.id == id)
    }
    pub(super) fn retain_records<F>(&mut self, predicate: F)
    where
        F: FnMut(&FangRecord) -> bool,
    {
        self.fangs.retain(predicate);
    }
    pub(super) fn terminate_peer(&mut self, peer: &str) -> usize {
        let ids: Vec<String> = self
            .fangs
            .iter()
            .filter(|fang| fang.peer == peer)
            .map(|fang| fang.id.clone())
            .collect();

        for id in &ids {
            if let Some(handle) = self.tasks.remove(id) {
                handle.abort();
            }
            if let Some(cancellation) = self.cancellations.remove(id) {
                cancellation.abort_children();
            }
            self.started.remove(id);
        }
        self.fangs.retain(|fang| fang.peer != peer);
        ids.len()
    }
    pub(super) fn insert_task(&mut self, id: String, handle: JoinHandle<()>) {
        self.tasks.insert(id, handle);
    }
    pub(super) fn insert_started(&mut self, id: String, started: Instant) {
        self.started.insert(id, started);
    }
    pub(super) fn insert_cancellation(&mut self, id: String, cancellation: FangCancellation) {
        self.cancellations.insert(id, cancellation);
    }
    pub(super) fn remove_cancellation(&mut self, id: &str) -> Option<FangCancellation> {
        self.cancellations.remove(id)
    }
    pub(super) fn remove_task(&mut self, id: &str) -> Option<JoinHandle<()>> {
        self.tasks.remove(id)
    }
    pub(super) fn remove_started(&mut self, id: &str) {
        self.started.remove(id);
    }
    pub(super) fn finished_ids(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|(_, handle)| handle.is_finished())
            .map(|(id, _)| id.clone())
            .collect()
    }
    pub(super) fn remove_finished(&mut self, dead_ids: &[String]) {
        for id in dead_ids {
            self.tasks.remove(id);
            self.started.remove(id);
            self.cancellations.remove(id);
        }
        self.fangs.retain(|f| !dead_ids.contains(&f.id));
    }
    pub(super) fn started_at(&self, id: &str) -> Option<&Instant> {
        self.started.get(id)
    }
    pub(super) fn abort_all(&mut self) {
        for cancellation in self.cancellations.values() {
            cancellation.abort_children();
        }
        for (_, handle) in self.tasks.drain() {
            handle.abort();
        }
        self.cancellations.clear();
    }
    pub(super) fn clear_started(&mut self) {
        self.started.clear();
    }
    pub(super) fn clear_records(&mut self) {
        self.fangs.clear();
    }
}
