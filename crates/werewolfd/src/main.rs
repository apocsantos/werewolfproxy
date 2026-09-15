#[cfg(not(target_os = "linux"))]
compile_error!("secure local control currently requires Linux");
mod admission;
mod control;
mod fang_registry;
mod handshake;
mod lifecycle;
mod policy;
mod protected_state;
mod state;
#[cfg(test)]
mod target_authorization_tests;
mod target_policy;
// Shared TLS plumbing is staged before either production transport adopts it.
#[allow(dead_code)]
mod tls_identity;
mod transport;
use fang_registry::FangCancellation;
use state::DaemonState;
mod cli;
use cli::Args;
mod quic_fang;
use crate::quic_fang::open_quic_fang;
#[cfg(test)]
mod quic_lab;
use clap::Parser;
use rand_core::{OsRng, RngCore};
use serde_json::json;
use std::{io, process::ExitCode, sync::Arc, time::Duration};
use tokio::{
    sync::{oneshot, watch, Mutex},
    task::{JoinHandle, JoinSet},
};
use werewolf_core::{
    fang::{FangRecord, FangState},
    protocol::ControlResponse,
    state::WolfMode,
};

const WEREWOLF_VERSION: &str = "v0.1.0-rc1";
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

macro_rules! ww_info {
    ($subsystem:expr, $event:expr, $($arg:tt)*) => {
        println!("[INFO][{}][{}] {}", $subsystem, $event, format!($($arg)*));
    };
}

mod config;
use config::{configure_den, expand_home, validate_startup_config};
use policy::{is_plain_tcp, select_peer_transport, ExpectedPeerIdentity, TransportPolicyError};

mod persistence;
use persistence::load_startup_state;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt::init();

    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(ExitClass::Configuration.code());
        }
    };
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            lifecycle_state(LifecycleState::Failed);
            eprintln!("[ERROR][SYSTEM][{}] {}", error.class.label(), error.message);
            ExitCode::from(error.class.code())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifecycleState {
    Starting,
    Validating,
    Locked,
    Ready,
    Stopping,
    Failed,
}

impl LifecycleState {
    fn label(self) -> &'static str {
        match self {
            Self::Starting => "STARTING",
            Self::Validating => "VALIDATING",
            Self::Locked => "LOCKED",
            Self::Ready => "READY",
            Self::Stopping => "STOPPING",
            Self::Failed => "FAILED",
        }
    }
}

fn lifecycle_state(state: LifecycleState) {
    ww_info!("SYSTEM", "STATE", "{}", state.label());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExitClass {
    Configuration,
    SecurityState,
    AlreadyRunning,
    Listener,
    Runtime,
}

impl ExitClass {
    fn code(self) -> u8 {
        match self {
            Self::Configuration => 64,
            Self::SecurityState => 65,
            Self::AlreadyRunning => 66,
            Self::Listener => 69,
            Self::Runtime => 70,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Configuration => "CONFIGURATION",
            Self::SecurityState => "SECURITY_STATE",
            Self::AlreadyRunning => "ALREADY_RUNNING",
            Self::Listener => "LISTENER",
            Self::Runtime => "RUNTIME",
        }
    }
}

struct DaemonFailure {
    class: ExitClass,
    message: String,
}

impl DaemonFailure {
    fn new(class: ExitClass, error: impl std::fmt::Display) -> Self {
        Self {
            class,
            message: error.to_string(),
        }
    }
}

async fn run(mut args: Args) -> Result<(), DaemonFailure> {
    lifecycle_state(LifecycleState::Starting);

    if args.socket.is_empty() {
        args.socket = werewolf_core::local_fs::default_control_socket()
            .map_err(|error| DaemonFailure::new(ExitClass::Configuration, error))?
            .into_os_string()
            .into_string()
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "non-UTF-8 control path")
            })
            .map_err(|error| DaemonFailure::new(ExitClass::Configuration, error))?;
    }

    ww_info!(
        "SYSTEM",
        "START",
        "🐺 WerewolfProxy {} starting",
        WEREWOLF_VERSION
    );

    let home = expand_home(&args.home);

    let mut initial_state = DaemonState::default();
    configure_den(&mut initial_state, &args, &home);

    lifecycle_state(LifecycleState::Validating);
    let den = Arc::new(
        werewolf_core::local_fs::PrivateDirectory::open(&home, true)
            .map_err(|error| DaemonFailure::new(ExitClass::SecurityState, error))?,
    );
    let _den_lock = den
        .lock(std::ffi::OsStr::new(".den.lock"))
        .map_err(|error| {
            let class = if error.kind() == io::ErrorKind::WouldBlock {
                ExitClass::AlreadyRunning
            } else {
                ExitClass::SecurityState
            };
            DaemonFailure::new(class, error)
        })?;
    load_startup_state(&den, &mut initial_state)
        .map_err(|error| DaemonFailure::new(ExitClass::SecurityState, error))?;
    initial_state.den = Some(den.clone());

    // Build one ephemeral TLS representation of the existing Pelt per process
    // startup. It remains in memory for later transport integration.
    initial_state
        .initialize_runtime_tls_identity()
        .map_err(|error| DaemonFailure::new(ExitClass::SecurityState, error))?;

    validate_startup_config(&initial_state);

    let pelt_ready = initial_state.runtime_tls_identity.is_some();
    let state = Arc::new(Mutex::new(initial_state));

    let listener = control::bind_socket(&args.socket)
        .await
        .map_err(|error| DaemonFailure::new(ExitClass::SecurityState, error))?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut control_task = tokio::spawn(control::serve_until_shutdown(
        listener,
        state.clone(),
        home.clone(),
        shutdown_rx,
    ));

    let active_profiles = state.lock().await.active_profiles.clone();

    if !active_profiles.is_empty() {
        ww_info!(
            "FANG",
            "RESTORE_COUNT",
            "🦷 Restoring {} active Fang profile(s)",
            active_profiles.len()
        );
    }

    for profile_name in active_profiles {
        let profile = {
            let st = state.lock().await;
            st.fang_profiles
                .iter()
                .find(|p| p.name == profile_name)
                .cloned()
        };

        match profile {
            Some(profile) => {
                ww_info!(
                    "FANG",
                    "RESTORE_PROFILE",
                    "🦷 Restoring active Fang profile: {}",
                    profile.name
                );

                let response = open_fang_from_parts(
                    "restore".to_string(),
                    state.clone(),
                    profile.peer,
                    profile.local,
                    profile.remote,
                    profile.transport,
                )
                .await;

                if !response.ok {
                    eprintln!("⚠️ Failed to restore Fang profile: {}", profile_name);
                }
            }
            None => {
                eprintln!("⚠️ Active Fang profile not found: {}", profile_name);
            }
        }
    }

    let (tcp_ready_tx, tcp_ready_rx) = oneshot::channel();
    let (quic_ready_tx, quic_ready_rx) = oneshot::channel();
    let mut transport_tasks = JoinSet::new();
    let net_state = state.clone();
    let listen_addr = args.listen.clone();
    transport_tasks.spawn(async move {
        transport::run_fang_listener_with_ready(&listen_addr, net_state, tcp_ready_tx).await
    });
    let quic_listen_addr = args.quic_listen.clone();
    let quic_state = state.clone();
    transport_tasks.spawn(async move {
        transport::run_quic_fang_listener_with_ready(&quic_listen_addr, quic_state, quic_ready_tx)
            .await
    });

    // With an existing Pelt, secure transport binding is a startup prerequisite.
    // With no Pelt, Stage12D4 deliberately leaves secure listeners unbound until
    // the local pelt.init transaction publishes a RuntimeTlsIdentity.
    if pelt_ready {
        if let Err(error) = await_listener_ready(tcp_ready_rx).await {
            shutdown_owned_tasks(
                &state,
                &shutdown_tx,
                &mut transport_tasks,
                Some(&mut control_task),
            )
            .await;
            return Err(DaemonFailure::new(ExitClass::Listener, error));
        }
        if let Err(error) = await_listener_ready(quic_ready_rx).await {
            shutdown_owned_tasks(
                &state,
                &shutdown_tx,
                &mut transport_tasks,
                Some(&mut control_task),
            )
            .await;
            return Err(DaemonFailure::new(ExitClass::Listener, error));
        }
    }

    ww_info!(
        "CONTROL",
        "SOCKET_READY",
        "🐺 werewolfd control socket: {}",
        args.socket
    );
    if pelt_ready {
        ww_info!(
            "FANG",
            "TCP_LISTENER",
            "🦷 Fang network listener: tcp://{}",
            args.listen
        );
        ww_info!(
            "QUIC",
            "LISTENER",
            "⚡ QUIC Fang listener: quic://{}",
            args.quic_listen
        );
    } else {
        ww_info!(
            "SYSTEM",
            "WAITING_FOR_PELT",
            "secure listeners remain unbound until local pelt.init succeeds"
        );
    }
    ww_info!("DEN", "HOME", "🏠 Den home: {}", home.display());

    let authority_locked = state.lock().await.inbound_authority.is_locked();
    if authority_locked {
        lifecycle_state(LifecycleState::Locked);
    } else if pelt_ready {
        lifecycle_state(LifecycleState::Ready);
    }

    let signal = lifecycle::wait_for_shutdown();
    tokio::pin!(signal);
    tokio::select! {
        reason = &mut signal => {
            ww_info!("SYSTEM", "SHUTDOWN_SIGNAL", "{} received", reason.label());
            shutdown_owned_tasks(
                &state,
                &shutdown_tx,
                &mut transport_tasks,
                Some(&mut control_task),
            ).await;
            Ok(())
        }
        result = &mut control_task => {
            shutdown_owned_tasks(&state, &shutdown_tx, &mut transport_tasks, None).await;
            result
                .map_err(|error| DaemonFailure::new(ExitClass::Runtime, error))?
                .map_err(|error| DaemonFailure::new(ExitClass::Runtime, error))?;
            Ok(())
        }
        result = transport_tasks.join_next(), if !transport_tasks.is_empty() => {
            let error = match result {
                Some(Ok(Ok(()))) => io::Error::other("secure listener stopped unexpectedly"),
                Some(Ok(Err(error))) => error,
                Some(Err(error)) => io::Error::other(error.to_string()),
                None => io::Error::other("all secure listener supervisors stopped"),
            };
            shutdown_owned_tasks(&state, &shutdown_tx, &mut transport_tasks, Some(&mut control_task)).await;
            Err(DaemonFailure::new(ExitClass::Listener, error))
        }
    }
}

async fn await_listener_ready(ready: oneshot::Receiver<io::Result<()>>) -> io::Result<()> {
    ready
        .await
        .map_err(|_| io::Error::other("secure listener stopped before binding"))?
}

async fn shutdown_owned_tasks(
    state: &Arc<Mutex<DaemonState>>,
    shutdown: &watch::Sender<bool>,
    transports: &mut JoinSet<io::Result<()>>,
    control: Option<&mut JoinHandle<io::Result<()>>>,
) {
    lifecycle_state(LifecycleState::Stopping);
    let authority = state.lock().await.inbound_authority.clone();
    let _ = authority.lock();
    {
        let mut state = state.lock().await;
        state.fang_registry.abort_all();
        state.fang_registry.clear_started();
        state.fang_registry.clear_records();
        state.status.active_fangs = 0;
    }
    let _ = shutdown.send(true);
    transports.abort_all();
    let _ = tokio::time::timeout(SHUTDOWN_DEADLINE, async {
        while transports.join_next().await.is_some() {}
    })
    .await;
    if let Some(control) = control {
        if tokio::time::timeout(SHUTDOWN_DEADLINE, &mut *control)
            .await
            .is_err()
        {
            control.abort();
            let _ = control.await;
        }
    }
}

fn generate_fang_id(peer: &str, local: &str, remote: &str, existing_count: usize) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let mut random = [0u8; 16];
    OsRng.fill_bytes(&mut random);

    let seed = format!(
        "{}|{}|{}|{}|{}|{:02x?}",
        now, existing_count, peer, local, remote, random
    );

    let hash = blake3::hash(seed.as_bytes());
    let hex = hash.to_hex();

    format!("fang_{}", &hex[..16])
}

async fn open_fang_from_parts(
    req_id: String,
    state: Arc<Mutex<DaemonState>>,
    peer: String,
    local: String,
    remote: String,
    transport: String,
) -> ControlResponse {
    open_fang_with_profile(req_id, state, peer, local, remote, transport, None).await
}

async fn open_fang_with_profile(
    req_id: String,
    state: Arc<Mutex<DaemonState>>,
    peer: String,
    local: String,
    remote: String,
    transport: String,
    activation: Option<(std::path::PathBuf, String)>,
) -> ControlResponse {
    let st = state.lock().await;

    if matches!(st.status.mode, WolfMode::Silver) {
        return ControlResponse::err(
            req_id,
            "SILVER_ACTIVE",
            "Silver mode is active. Fang open rejected.",
        );
    }

    if peer.is_empty() || local.is_empty() || remote.is_empty() {
        return ControlResponse::err(
            req_id,
            "FANG_INVALID",
            "peer, local and remote are required",
        );
    }

    if st.fang_registry.has_active_local(&local) {
        return ControlResponse::err(
            req_id,
            "FANG_ALREADY_ACTIVE",
            format!("Fang already active on local address: {}", local),
        );
    }

    let peer_record = match st.peers.iter().find(|p| p.name == peer) {
        Some(p) => p.clone(),
        None => {
            return ControlResponse::err(
                req_id,
                "FANG_UNKNOWN_PEER",
                format!("Unknown peer: {}", peer),
            )
        }
    };

    let identity = match &st.pelt {
        Some(pelt) => pelt.clone(),
        None => return ControlResponse::err(req_id, "NO_PELT", "No local Pelt identity exists"),
    };
    let expected_peer_identity = ExpectedPeerIdentity {
        fingerprint: peer_record.fingerprint.clone(),
        public_key_b64: peer_record.public_key_b64.clone(),
    };

    let peer_addr = match select_peer_transport(&transport, &peer_record.address) {
        Ok(transport::FangTransport::Quic(a)) => {
            let fang_id = generate_fang_id(&peer, &local, &remote, st.fang_registry.len());

            let (cancellation, release) = FangCancellation::prepared();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let handle = match open_quic_fang(
                local.clone(),
                a.clone(),
                remote.clone(),
                identity.clone(),
                cancellation.clone(),
                ready_tx,
                expected_peer_identity.clone(),
            )
            .await
            {
                Ok(h) => h,
                Err(e) => {
                    return ControlResponse::err(req_id, "QUIC_FANG_FAILED", e.to_string());
                }
            };

            match ready_rx.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    handle.abort();
                    return ControlResponse::err(req_id, "FANG_LISTENER_FAILED", error.to_string());
                }
                Err(_) => {
                    handle.abort();
                    return ControlResponse::err(
                        req_id,
                        "FANG_LISTENER_FAILED",
                        "Fang listener stopped before becoming ready",
                    );
                }
            }

            drop(st);
            if persist_activation(&activation, &state).await.is_err() {
                handle.abort();
                return ControlResponse::err(req_id, "ACTIVE_SAVE_FAILED", "activation rejected");
            }
            let mut st = state.lock().await;

            let fang = FangRecord {
                id: fang_id.clone(),
                peer: peer.clone(),
                local: local.clone(),
                remote: remote.clone(),
                state: FangState::Active,
            };

            st.fang_registry.push(fang);
            st.fang_registry.insert_task(fang_id.clone(), handle);
            st.fang_registry
                .insert_cancellation(fang_id.clone(), cancellation);
            st.fang_registry
                .insert_started(fang_id.clone(), std::time::Instant::now());

            st.status.active_fangs = st.fang_registry.len();
            st.status.mode = WolfMode::Wolf;

            if release.send(true).is_err() {
                st.storage_degraded = true;
                return ControlResponse::err(
                    req_id,
                    "STORAGE_DEGRADED",
                    "activation publication failed",
                );
            }

            return ControlResponse::ok(
                req_id,
                json!({
                    "fang_id": fang_id,
                    "peer": peer,
                    "local": local,
                    "remote": remote,
                    "state": "active",
                    "transport": "quic",
                    "quic_server": a
                }),
            );
        }
        Ok(transport::FangTransport::Tcp(a)) => a,
        Err(TransportPolicyError::BadTcpAddress) => {
            return ControlResponse::err(req_id, "FANG_BAD_TCP_ADDRESS", "Peer TCP address invalid")
        }
        Err(TransportPolicyError::BadQuicAddress) => {
            return ControlResponse::err(
                req_id,
                "FANG_BAD_QUIC_ADDRESS",
                "Peer QUIC address invalid",
            )
        }
    };

    let fang_id = generate_fang_id(&peer, &local, &remote, st.fang_registry.len());

    let fang = FangRecord {
        id: fang_id.clone(),
        peer: peer.clone(),
        local: local.clone(),
        remote: remote.clone(),
        state: FangState::Active,
    };

    let task_fang_id = fang_id.clone();
    let task_local = local.clone();
    let task_peer_addr = peer_addr.clone();
    let task_remote = remote.clone();
    let task_identity = identity.clone();
    let task_transport = transport.clone();
    let (cancellation, release) = FangCancellation::prepared();
    let task_cancellation = cancellation.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let task_expected_peer = expected_peer_identity;

    let handle = tokio::spawn(async move {
        let result = transport::run_selected_forwarder(
            &task_fang_id,
            &task_local,
            &task_peer_addr,
            &task_remote,
            task_identity,
            is_plain_tcp(&task_transport),
            task_cancellation,
            ready_tx,
            task_expected_peer,
        )
        .await;

        if let Err(e) = result {
            eprintln!("fang {} failed: {}", task_fang_id, e);
        }
    });

    match ready_rx.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            handle.abort();
            return ControlResponse::err(req_id, "FANG_LISTENER_FAILED", error.to_string());
        }
        Err(_) => {
            handle.abort();
            return ControlResponse::err(
                req_id,
                "FANG_LISTENER_FAILED",
                "Fang listener stopped before becoming ready",
            );
        }
    }

    drop(st);
    if persist_activation(&activation, &state).await.is_err() {
        handle.abort();
        return ControlResponse::err(req_id, "ACTIVE_SAVE_FAILED", "activation rejected");
    }
    let mut st = state.lock().await;
    st.fang_registry.push(fang);
    st.status.active_fangs = st.fang_registry.len();
    st.status.mode = WolfMode::Wolf;

    st.fang_registry.insert_task(fang_id.clone(), handle);
    st.fang_registry
        .insert_cancellation(fang_id.clone(), cancellation);
    st.fang_registry
        .insert_started(fang_id.clone(), std::time::Instant::now());

    if release.send(true).is_err() {
        st.storage_degraded = true;
        return ControlResponse::err(req_id, "STORAGE_DEGRADED", "activation publication failed");
    }

    ControlResponse::ok(
        req_id,
        json!({
            "fang_id": fang_id,
            "peer": peer,
            "local": local,
            "remote": remote,
            "state": "active",
            "transport": transport,
            "peer_addr": peer_addr
        }),
    )
}

async fn persist_activation(
    activation: &Option<(std::path::PathBuf, String)>,
    state: &Arc<Mutex<DaemonState>>,
) -> std::io::Result<()> {
    if let Some((home, name)) = activation {
        let mut candidate = state.lock().await.active_profiles.clone();
        if !candidate.contains(name) {
            candidate.push(name.clone());
        }
        control::persist_active(home, candidate, state).await?;
    }
    Ok(())
}

#[cfg(test)]
mod replay_v3_tests;

#[cfg(test)]
mod authority_tests;

mod authority;
