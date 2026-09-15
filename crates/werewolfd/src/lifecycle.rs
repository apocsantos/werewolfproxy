//! Linux foreground-service signal handling. Service readiness is deliberately
//! log-based in Stage17: Type=notify would require a new systemd dependency.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShutdownSignal {
    Interrupt,
    Terminate,
}

impl ShutdownSignal {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
        }
    }
}

/// SIGINT and SIGTERM deliberately take the same orderly shutdown path.
/// SIGHUP is retained and reported as an explicit no-op: persistent state is
/// changed only through the already-transactional local control API.
pub(super) async fn wait_for_shutdown() -> ShutdownSignal {
    use tokio::signal::unix::{signal, SignalKind};

    let mut interrupt = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    let mut terminate = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut hangup = signal(SignalKind::hangup()).expect("install SIGHUP handler");
    loop {
        tokio::select! {
            _ = interrupt.recv() => return ShutdownSignal::Interrupt,
            _ = terminate.recv() => return ShutdownSignal::Terminate,
            _ = hangup.recv() => {
                eprintln!("[INFO][SYSTEM][SIGHUP_IGNORED] live reload is not supported; use werewolfctl transactions");
            }
        }
    }
}
