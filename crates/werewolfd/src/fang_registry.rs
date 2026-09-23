use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::task::JoinHandle;
use werewolf_core::fang::FangRecord;

struct ChildTasks {
    accepting: bool,
    handles: Vec<JoinHandle<()>>,
    #[cfg(test)]
    close_observed: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Default for ChildTasks {
    fn default() -> Self {
        Self {
            accepting: true,
            handles: Vec::new(),
            #[cfg(test)]
            close_observed: None,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct FangCancellation {
    children: Arc<Mutex<ChildTasks>>,
    activation: Option<tokio::sync::watch::Receiver<bool>>,
}

impl FangCancellation {
    pub(super) fn prepared() -> (Self, tokio::sync::watch::Sender<bool>) {
        let (release, activation) = tokio::sync::watch::channel(false);
        (
            Self {
                activation: Some(activation),
                ..Self::default()
            },
            release,
        )
    }

    pub(super) async fn await_activation(&self) -> std::io::Result<()> {
        if let Some(mut activation) = self.activation.clone() {
            activation
                .wait_for(|released| *released)
                .await
                .map_err(|_| std::io::Error::other("listener preparation cancelled"))?;
        }
        Ok(())
    }

    /// Admission and registration are one synchronous operation. Detachment
    /// closes admission under this same mutex, so no child can escape the
    /// subsequent listener-then-children join barrier.
    pub(super) fn spawn<F>(&self, future: F) -> bool
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let mut children = self.children.lock().unwrap();
        if !children.accepting {
            return false;
        }
        children.handles.retain(|child| !child.is_finished());
        children.handles.push(tokio::spawn(future));
        true
    }

    fn close_admission(&self) {
        let mut children = self.children.lock().unwrap();
        children.accepting = false;
        #[cfg(test)]
        if let Some(observed) = children.close_observed.take() {
            let _ = observed.send(());
        }
    }

    #[cfg(test)]
    pub(super) fn observe_close(&self, observed: tokio::sync::oneshot::Sender<()>) {
        self.children.lock().unwrap().close_observed = Some(observed);
    }

    fn take_children(&self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.children.lock().unwrap().handles)
    }

    pub(super) fn abort_children(&self) {
        self.close_admission();
        for handle in self.take_children() {
            handle.abort();
        }
    }
}

/// Owned after Fang records have been detached from the registry. Awaiting
/// this bundle never requires the DaemonState mutex.
pub(super) struct FangTermination {
    listeners: Vec<JoinHandle<()>>,
    cancellations: Vec<FangCancellation>,
    closed: usize,
}

impl FangTermination {
    pub(super) fn closed(&self) -> usize {
        self.closed
    }

    pub(super) async fn quiesce(mut self) {
        for listener in &self.listeners {
            listener.abort();
        }
        for listener in self.listeners.drain(..) {
            let _ = listener.await;
        }
        // A listener that was already inside accept/spawn has now exited.
        // Every child it created is in this shared tracker.
        let mut children = Vec::new();
        for cancellation in &self.cancellations {
            children.extend(cancellation.take_children());
        }
        for child in &children {
            child.abort();
        }
        for child in children {
            let _ = child.await;
        }
    }
}

impl Drop for FangTermination {
    fn drop(&mut self) {
        for listener in &self.listeners {
            listener.abort();
        }
        for cancellation in &self.cancellations {
            cancellation.abort_children();
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
    pub(super) fn terminate_ids(&mut self, ids: &[String]) -> usize {
        for id in ids {
            if let Some(handle) = self.tasks.remove(id) {
                handle.abort();
            }
            if let Some(cancellation) = self.cancellations.remove(id) {
                cancellation.abort_children();
            }
            self.started.remove(id);
        }
        self.fangs.retain(|fang| !ids.contains(&fang.id));
        ids.len()
    }
    pub(super) fn detach_ids(&mut self, ids: &[String]) -> FangTermination {
        let mut termination = FangTermination {
            listeners: Vec::new(),
            cancellations: Vec::new(),
            closed: 0,
        };
        for id in ids {
            if self.fangs.iter().any(|fang| &fang.id == id) {
                termination.closed += 1;
            }
            if let Some(cancellation) = self.cancellations.remove(id) {
                cancellation.close_admission();
                termination.cancellations.push(cancellation);
            }
            if let Some(listener) = self.tasks.remove(id) {
                termination.listeners.push(listener);
            }
            self.started.remove(id);
        }
        self.fangs.retain(|fang| !ids.contains(&fang.id));
        termination
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

#[cfg(test)]
mod activation_tests {
    use super::*;
    #[tokio::test]
    async fn preparation_waits_for_publication_and_fails_on_abandonment() {
        let (prepared, release) = FangCancellation::prepared();
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(20),
            prepared.await_activation()
        )
        .await
        .is_err());
        release.send(true).unwrap();
        prepared.await_activation().await.unwrap();
        let (abandoned, release) = FangCancellation::prepared();
        drop(release);
        assert!(abandoned.await_activation().await.is_err());
        FangCancellation::default()
            .await_activation()
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn tracked_children_can_be_aborted_and_drained() {
        let cancellation = FangCancellation::default();
        for _ in 0..4096 {
            assert!(cancellation.spawn(async {}));
        }
        cancellation.abort_children();
        assert!(cancellation.children.lock().unwrap().handles.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn quiescence_waits_for_existing_child_and_leaves_other_fang_open() {
        use std::sync::Condvar;
        let cancellation = FangCancellation::default();
        let other = FangCancellation::default();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let child_gate = gate.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        assert!(cancellation.spawn(async move {
            let _ = entered_tx.send(());
            let (lock, ready) = &*child_gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = ready.wait(released).unwrap();
            }
        }));
        entered_rx.await.unwrap();

        let mut registry = FangRegistry::default();
        for (id, peer) in [("old", "old"), ("other", "other")] {
            registry.push(FangRecord {
                id: id.into(),
                peer: peer.into(),
                local: "127.0.0.1:1".into(),
                remote: "127.0.0.1:2".into(),
                state: werewolf_core::fang::FangState::Active,
            });
        }
        registry.insert_cancellation("old".into(), cancellation);
        registry.insert_cancellation("other".into(), other.clone());
        let termination = registry.detach_ids(&["old".into()]);
        assert_eq!(termination.closed(), 1);
        assert_eq!(registry.records()[0].peer, "other");
        let quiesce = tokio::spawn(termination.quiesce());
        assert!(!quiesce.is_finished());
        {
            let (lock, ready) = &*gate;
            *lock.lock().unwrap() = true;
            ready.notify_one();
        }
        quiesce.await.unwrap();
        assert!(other.spawn(async {}));
        other.abort_children();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn detached_listener_rejects_late_child_and_quiescence_waits_for_exit() {
        use std::sync::Condvar;
        let cancellation = FangCancellation::default();
        let listener_cancellation = cancellation.clone();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let listener_gate = gate.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (spawned_tx, spawned_rx) = tokio::sync::oneshot::channel();
        let listener = tokio::spawn(async move {
            let _ = entered_tx.send(());
            let (lock, ready) = &*listener_gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = ready.wait(released).unwrap();
            }
            let _ = spawned_tx.send(listener_cancellation.spawn(async {}));
        });
        entered_rx.await.unwrap();

        let mut registry = FangRegistry::default();
        registry.push(FangRecord {
            id: "fang".into(),
            peer: "peer".into(),
            local: "127.0.0.1:1".into(),
            remote: "127.0.0.1:2".into(),
            state: werewolf_core::fang::FangState::Active,
        });
        registry.insert_task("fang".into(), listener);
        registry.insert_cancellation("fang".into(), cancellation);
        let termination = registry.detach_ids(&["fang".into()]);
        assert_eq!(termination.closed(), 1);
        assert!(registry.is_empty());
        let quiesce = tokio::spawn(termination.quiesce());
        assert!(!quiesce.is_finished());
        {
            let (lock, ready) = &*gate;
            *lock.lock().unwrap() = true;
            ready.notify_one();
        }
        assert!(!spawned_rx.await.unwrap());
        quiesce.await.unwrap();
    }
}
