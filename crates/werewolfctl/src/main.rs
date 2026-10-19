#[cfg(not(target_os = "linux"))]
compile_error!("secure local control currently requires Linux");

use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::{ffi::OsStr, io, path::PathBuf, process::ExitCode};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};
use werewolf_core::{
    local_fs::PrivateDirectory,
    pelt::{fingerprint_from_public_key_b64, PeltIdentity},
    protocol::{ControlRequest, ControlResponse},
    state_validation,
};

// A release build supplies a fixed external RC version; ordinary developer
// builds deliberately remain free of timestamp-derived version metadata.
const VERSION: &str = match option_env!("WEREWOLF_RELEASE_VERSION") {
    Some(version) => version,
    None => "v0.1.0-rc1",
};

#[derive(Parser)]
#[command(
    name = "werewolfctl",
    about = "WerewolfProxy local administration tool",
    version = VERSION
)]
struct Cli {
    /// Explicit Unix control socket. It takes precedence over the runtime default.
    #[arg(long, default_value = "")]
    socket: String,
    /// Den used only by offline commands. The daemon is authoritative online.
    #[arg(long, default_value = "~/.config/werewolf")]
    home: String,
    /// Emit one bounded JSON object on stdout.
    #[arg(long)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Version,
    Status,
    Health,
    Diagnose,
    Doctor,
    Den {
        #[command(subcommand)]
        command: DenCommands,
    },
    State {
        #[command(subcommand)]
        command: StateCommands,
    },
    Silver {
        #[command(subcommand)]
        command: SilverCommands,
    },
    Pelt {
        #[command(subcommand)]
        command: PeltCommands,
    },
    Pack {
        #[command(subcommand)]
        command: PackCommands,
    },
    Target {
        #[command(subcommand)]
        command: TargetCommands,
    },
    Fang {
        #[command(subcommand)]
        command: FangCommands,
    },
    /// User-facing service connections over Werewolf.
    Service {
        #[command(subcommand)]
        command: ServiceCommands,
    },
    /// Find Pack members' contacts. Discovery never adds anyone to the Pack.
    Discover {
        #[command(subcommand)]
        command: DiscoverCommands,
    },
}
#[derive(Subcommand)]
enum DenCommands {
    Init,
    Info,
}
#[derive(Subcommand)]
enum StateCommands {
    ManifestMigrate,
    BackupInfo,
    /// Compatibility-only complete document replacement. Prefer target allow/remove.
    #[command(hide = true)]
    SetTargetPolicy {
        document: String,
    },
}
#[derive(Subcommand)]
enum SilverCommands {
    Status,
    On,
    Off,
    #[command(hide = true)]
    Trigger,
    #[command(hide = true)]
    Reset,
}
#[derive(Subcommand)]
enum PeltCommands {
    Init,
    Show,
    #[command(hide = true)]
    Fingerprint,
}
#[derive(Subcommand)]
enum PackCommands {
    /// Add a Pack peer from its public Pelt key and transport address.
    Add {
        name: String,
        public_key_b64: String,
        address: String,
    },
    List,
    Remove {
        name: String,
    },
    Revoke {
        name: String,
    },
    SetAddress {
        name: String,
        address: String,
    },
}
#[derive(Subcommand)]
enum TargetCommands {
    /// Allow one exact IP:port target for this Pack peer name or fingerprint.
    Allow {
        peer: String,
        target: String,
    },
    List,
    Remove {
        peer: String,
        target: String,
    },
}
#[derive(Subcommand)]
enum FangCommands {
    /// Create a persistent profile without opening a listener.
    Create {
        name: String,
        peer: String,
        local: String,
        remote: String,
        #[arg(long, default_value = "quic", value_parser = ["quic", "tcp", "tcp-plain"])]
        transport: String,
    },
    /// List configured persistent profiles.
    List,
    /// List running Fangs.
    Active,
    Activate {
        name: String,
    },
    Deactivate {
        name: String,
    },
    /// Active profiles must be deactivated before removal.
    Remove {
        name: String,
    },
    #[command(hide = true)]
    Open {
        peer: String,
        local: String,
        remote: String,
    },
    #[command(hide = true)]
    OpenProfile {
        name: String,
    },
    #[command(hide = true)]
    Close {
        fang_id: String,
    },
    #[command(hide = true)]
    Cleanup,
}

#[derive(Subcommand)]
enum ServiceCommands {
    /// Define a named service. It records a destination; it grants no access.
    Add {
        name: String,
        /// Peer name; it need not be in the Pack yet.
        peer: String,
        /// Target on the peer, for example 127.0.0.1:8080.
        target: String,
        #[arg(value_parser = ["tcp", "http", "https", "ssh"])]
        kind: String,
    },
    List,
    Show {
        name: String,
    },
    Remove {
        name: String,
    },
    /// Connect a local listener to a named service through the normal Fang path.
    Connect {
        /// Service name from `service list`.
        name: String,

        /// Local TCP port to expose.
        #[arg(long)]
        local_port: u16,

        /// Local bind address.
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,

        #[arg(long, default_value = "quic", value_parser = ["quic", "tcp", "tcp-plain"])]
        transport: String,
    },
}

#[derive(Subcommand)]
enum DiscoverCommands {
    /// Publish where this node is reachable (1 to 4 quic:// or tcp:// IP:port).
    Advertise {
        #[arg(required = true, num_args = 1..=4)]
        addresses: Vec<String>,
    },
    /// Stop advertising an address.
    Withdraw,
    /// Show this node's ID and signed record.
    Show,
    /// List cached contact records.
    Contacts,
    /// Ask every Pack member for its own record.
    Refresh {
        #[arg(long, default_value = "quic", value_parser = ["quic", "tcp"])]
        transport: String,
    },
    /// Ask the nearest Pack members for a node by public key or node ID.
    Find {
        /// Public key (base64) or node ID (64 hex digits).
        node: String,
        #[arg(long, default_value = "quic", value_parser = ["quic", "tcp"])]
        transport: String,
    },
}

#[derive(Clone, Copy)]
enum ExitClass {
    Usage = 64,
    State = 65,
    Conflict = 66,
    DaemonUnavailable = 69,
    Rejected = 70,
}
struct CliError {
    class: ExitClass,
    message: String,
}
impl CliError {
    fn new(class: ExitClass, message: impl std::fmt::Display) -> Self {
        Self {
            class,
            message: message.to_string(),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let json_output = cli.json;
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("werewolfctl: {}", error.message);
            if json_output {
                println!(
                    "{}",
                    serde_json::to_string(
                        &json!({"ok":false,"error":error.message,"exit_class":error.class as u8})
                    )
                    .unwrap()
                );
            }
            ExitCode::from(error.class as u8)
        }
    }
}

async fn run(cli: Cli) -> Result<(), CliError> {
    if matches!(cli.command, Commands::Version) {
        return print_local(
            cli.json,
            "version",
            json!({"version":VERSION,"commit":option_env!("WEREWOLF_GIT_COMMIT").unwrap_or("unknown")}),
        );
    }
    if matches!(
        cli.command,
        Commands::Den {
            command: DenCommands::Init
        }
    ) {
        let home = expand_home(&cli.home);
        let den =
            PrivateDirectory::open(&home, true).map_err(|e| CliError::new(ExitClass::State, e))?;
        let _lock = den
            .lock(OsStr::new(".den.lock"))
            .map_err(|e| CliError::new(ExitClass::Conflict, e))?;
        return print_local(
            cli.json,
            "den.init",
            json!({"home":home,"status":"ready_for_pelt_init","pelt_created":false}),
        );
    }
    if matches!(
        cli.command,
        Commands::State {
            command: StateCommands::BackupInfo
        }
    ) {
        let home = expand_home(&cli.home);
        return print_local(cli.json, "state.backup-info", backup_info(home));
    }
    if matches!(cli.command, Commands::Doctor) {
        return doctor(&cli).await;
    }
    let (command, args) = request_for(cli.command)?;
    let socket = socket_path(&cli.socket)?;
    let response = send_request(&socket, &command, args)
        .await
        .map_err(|e| CliError::new(ExitClass::DaemonUnavailable, e))?;
    print_response(cli.json, &command, response)
}

fn request_for(command: Commands) -> Result<(String, Value), CliError> {
    let (cmd, args) = match command {
        Commands::Status | Commands::Health | Commands::Diagnose => ("status", json!({})),
        Commands::Den {
            command: DenCommands::Info,
        } => ("den.info", json!({})),
        Commands::State {
            command: StateCommands::ManifestMigrate,
        } => ("state.manifest.migrate", json!({})),
        Commands::State {
            command: StateCommands::SetTargetPolicy { document },
        } => ("target.policy.set", json!({"document":document})),
        Commands::Silver {
            command: SilverCommands::Status,
        } => ("status", json!({})),
        Commands::Silver {
            command: SilverCommands::On | SilverCommands::Trigger,
        } => ("silver.trigger", json!({})),
        Commands::Silver {
            command: SilverCommands::Off | SilverCommands::Reset,
        } => ("silver.reset", json!({})),
        Commands::Pelt {
            command: PeltCommands::Init,
        } => ("pelt.init", json!({})),
        Commands::Pelt {
            command: PeltCommands::Show | PeltCommands::Fingerprint,
        } => ("pelt.fingerprint", json!({})),
        Commands::Pack {
            command:
                PackCommands::Add {
                    name,
                    public_key_b64,
                    address,
                },
        } => {
            let fingerprint = fingerprint_from_public_key_b64(&public_key_b64).map_err(|_| {
                CliError::new(
                    ExitClass::Usage,
                    "public_key_b64 is not a valid public Ed25519 key",
                )
            })?;
            (
                "pack.add",
                json!({"name":name,"fingerprint":fingerprint,"public_key_b64":public_key_b64,"address":address}),
            )
        }
        Commands::Pack {
            command: PackCommands::List,
        } => ("pack.list", json!({})),
        Commands::Pack {
            command: PackCommands::Remove { name },
        } => ("pack.remove", json!({"name":name})),
        Commands::Pack {
            command: PackCommands::Revoke { name },
        } => ("pack.revoke", json!({"name":name})),
        Commands::Pack {
            command: PackCommands::SetAddress { name, address },
        } => ("pack.set_address", json!({"name":name,"address":address})),
        Commands::Target {
            command: TargetCommands::Allow { peer, target },
        } => ("target.allow", json!({"peer":peer,"target":target})),
        Commands::Target {
            command: TargetCommands::List,
        } => ("target.list", json!({})),
        Commands::Target {
            command: TargetCommands::Remove { peer, target },
        } => ("target.remove", json!({"peer":peer,"target":target})),
        Commands::Fang {
            command:
                FangCommands::Create {
                    name,
                    peer,
                    local,
                    remote,
                    transport,
                },
        } => (
            "fang.profile.add",
            json!({"name":name,"peer":peer,"local":local,"remote":remote,"transport":transport}),
        ),
        Commands::Fang {
            command: FangCommands::List,
        } => ("fang.profile.list", json!({})),
        Commands::Fang {
            command: FangCommands::Active,
        } => ("fang.list", json!({})),
        Commands::Fang {
            command: FangCommands::Activate { name } | FangCommands::OpenProfile { name },
        } => ("fang.open_profile", json!({"name":name})),
        Commands::Fang {
            command: FangCommands::Deactivate { name },
        } => ("fang.deactivate_profile", json!({"name":name})),
        Commands::Fang {
            command: FangCommands::Remove { name },
        } => ("fang.profile.remove", json!({"name":name})),
        Commands::Fang {
            command:
                FangCommands::Open {
                    peer,
                    local,
                    remote,
                },
        } => (
            "fang.open",
            json!({"peer":peer,"local":local,"remote":remote}),
        ),
        Commands::Fang {
            command: FangCommands::Close { fang_id },
        } => ("fang.close", json!({"fang_id":fang_id})),
        Commands::Fang {
            command: FangCommands::Cleanup,
        } => ("fang.cleanup", json!({})),
        Commands::Service {
            command:
                ServiceCommands::Add {
                    name,
                    peer,
                    target,
                    kind,
                },
        } => (
            "service.add",
            json!({"name":name,"peer":peer,"target":target,"kind":kind}),
        ),
        Commands::Service {
            command: ServiceCommands::List,
        } => ("service.list", json!({})),
        Commands::Service {
            command: ServiceCommands::Show { name },
        } => ("service.show", json!({"name":name})),
        Commands::Service {
            command: ServiceCommands::Remove { name },
        } => ("service.remove", json!({"name":name})),
        Commands::Service {
            command:
                ServiceCommands::Connect {
                    name,
                    local_port,
                    bind,
                    transport,
                },
        } => {
            // The daemon resolves the name to its registered peer and target.
            let local = format!("{bind}:{local_port}");
            (
                "service.connect",
                json!({"name":name,"local":local,"transport":transport}),
            )
        }
        Commands::Discover { command } => match command {
            DiscoverCommands::Advertise { addresses } => (
                "discover.advertise",
                json!({"addresses":addresses.join(",")}),
            ),
            DiscoverCommands::Withdraw => ("discover.advertise", json!({"addresses":""})),
            DiscoverCommands::Show => ("discover.show", json!({})),
            DiscoverCommands::Contacts => ("discover.contacts", json!({})),
            DiscoverCommands::Refresh { transport } => {
                ("discover.refresh", json!({"transport":transport}))
            }
            DiscoverCommands::Find { node, transport } => (
                "discover.find",
                json!({"target":node,"transport":transport}),
            ),
        },
        Commands::Version
        | Commands::Doctor
        | Commands::Den {
            command: DenCommands::Init,
        }
        | Commands::State {
            command: StateCommands::BackupInfo,
        } => unreachable!("handled locally"),
    };
    Ok((cmd.to_owned(), args))
}

fn socket_path(socket: &str) -> Result<String, CliError> {
    if !socket.is_empty() {
        return Ok(socket.to_owned());
    }
    werewolf_core::local_fs::default_control_socket()
        .map_err(|e| CliError::new(ExitClass::DaemonUnavailable, e))?
        .into_os_string()
        .into_string()
        .map_err(|_| CliError::new(ExitClass::Usage, "control socket path is not UTF-8"))
}
fn expand_home(input: &str) -> PathBuf {
    if input == "~" {
        return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(rest);
    }
    PathBuf::from(input)
}
fn backup_info(home: PathBuf) -> Value {
    json!({"home":home,"requires_daemon_stopped":true,"include":["pelt.json","security_state_manifest_mode.json","security_state_current.json","security_state_a_*","security_state_b_*"],"exclude":[".den.lock","control.sock","runtime"],"warning":"A complete historical Den remains acceptable without an external freshness anchor."})
}

async fn doctor(cli: &Cli) -> Result<(), CliError> {
    let home = expand_home(&cli.home);
    let den = PrivateDirectory::open(&home, false).map_err(|e| {
        CliError::new(
            ExitClass::State,
            format!("ERROR: Den validation failed: {e}"),
        )
    })?;
    let mut checks = vec![json!({"name":"den","level":"OK","detail":"secure Den path validated"})];
    let mode = den
        .read(OsStr::new("security_state_manifest_mode.json"), 4096)
        .map_err(|e| CliError::new(ExitClass::State, e))?;
    let current = den
        .read(OsStr::new("security_state_current.json"), 4096)
        .map_err(|e| CliError::new(ExitClass::State, e))?;
    match (mode, current) {
        (Some(mode), Some(current)) if serde_json::from_slice::<Value>(&mode).is_ok() && serde_json::from_slice::<Value>(&current).is_ok() => checks.push(json!({"name":"manifest-selector","level":"OK","detail":"manifest mode and selector present"})),
        (None, None) => checks.push(json!({"name":"manifest-selector","level":"WARNING","detail":"legacy Den; run state manifest-migrate while daemon is running"})),
        _ => return Err(CliError::new(ExitClass::State, "ERROR: incomplete or malformed Stage15B manifest selector")),
    }
    match den
        .read(OsStr::new("pelt.json"), 4096)
        .map_err(|e| CliError::new(ExitClass::State, e))?
    {
        None => {
            checks.push(json!({"name":"pelt","level":"WARNING","detail":"Pelt is not initialized"}))
        }
        Some(bytes) => {
            let pelt: PeltIdentity = serde_json::from_slice(&bytes)
                .map_err(|_| CliError::new(ExitClass::State, "ERROR: Pelt is malformed"))?;
            state_validation::identity(&pelt)
                .map_err(|_| CliError::new(ExitClass::State, "ERROR: Pelt validation failed"))?;
            checks.push(json!({"name":"pelt","level":"OK","detail":"Pelt public identity validates","fingerprint":pelt.fingerprint}));
        }
    }
    print_local(
        cli.json,
        "doctor",
        json!({"home":home,"checks":checks,"result":"OK_OR_WARNING"}),
    )
}

async fn send_request(socket: &str, cmd: &str, args: Value) -> io::Result<ControlResponse> {
    let mut stream = UnixStream::connect(socket).await?;
    let request = ControlRequest {
        id: "werewolfctl".into(),
        cmd: cmd.into(),
        args,
    };
    let bytes = serde_json::to_vec(&request).map_err(io::Error::other)?;
    stream.write_all(&bytes).await?;
    stream.write_all(b"\n").await?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    serde_json::from_str(&line)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid control response"))
}
fn print_response(
    json_output: bool,
    command: &str,
    response: ControlResponse,
) -> Result<(), CliError> {
    if !response.ok {
        let error = response
            .error
            .map(|error| error.message)
            .unwrap_or_else(|| "operation rejected".into());
        return Err(CliError::new(ExitClass::Rejected, error));
    }
    print_local(
        json_output,
        command,
        response.result.unwrap_or_else(|| json!({})),
    )
}
fn print_local(json_output: bool, command: &str, result: Value) -> Result<(), CliError> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string(&json!({"ok":true,"command":command,"result":result}))
                .map_err(io::Error::other)
                .map_err(|e| CliError::new(ExitClass::State, e))?
        );
    } else {
        println!("{command}");
        println!(
            "{}",
            serde_json::to_string_pretty(&result)
                .map_err(io::Error::other)
                .map_err(|e| CliError::new(ExitClass::State, e))?
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(args: &[&str]) -> (String, Value) {
        let cli = Cli::try_parse_from(std::iter::once("werewolfctl").chain(args.iter().copied()))
            .unwrap();
        match request_for(cli.command) {
            Ok(request) => request,
            Err(error) => panic!("{}", error.message),
        }
    }

    #[test]
    fn service_commands_map_to_daemon_requests() {
        assert_eq!(
            request(&["service", "add", "web", "wolf-a", "127.0.0.1:8080", "http"]),
            (
                "service.add".into(),
                json!({"name":"web","peer":"wolf-a","target":"127.0.0.1:8080","kind":"http"})
            )
        );
        assert_eq!(
            request(&["service", "list"]),
            ("service.list".into(), json!({}))
        );
        assert_eq!(
            request(&["service", "show", "web"]),
            ("service.show".into(), json!({"name":"web"}))
        );
        assert_eq!(
            request(&["service", "remove", "web"]),
            ("service.remove".into(), json!({"name":"web"}))
        );
    }

    #[test]
    fn discover_commands_map_to_control_requests() {
        assert_eq!(
            request(&[
                "discover",
                "advertise",
                "quic://192.0.2.1:9560",
                "tcp://192.0.2.1:9560"
            ]),
            (
                "discover.advertise".into(),
                json!({"addresses":"quic://192.0.2.1:9560,tcp://192.0.2.1:9560"})
            )
        );
        assert_eq!(
            request(&["discover", "withdraw"]),
            ("discover.advertise".into(), json!({"addresses":""}))
        );
        assert_eq!(
            request(&["discover", "find", "abcd", "--transport", "tcp"]),
            (
                "discover.find".into(),
                json!({"target":"abcd","transport":"tcp"})
            )
        );
        assert_eq!(
            request(&["discover", "refresh"]),
            ("discover.refresh".into(), json!({"transport":"quic"}))
        );
        for bad in [
            &["werewolfctl", "discover", "advertise"][..],
            &[
                "werewolfctl",
                "discover",
                "find",
                "x",
                "--transport",
                "tcp-plain",
            ],
            &[
                "werewolfctl",
                "discover",
                "advertise",
                "a",
                "b",
                "c",
                "d",
                "e",
            ],
        ] {
            assert!(Cli::try_parse_from(bad).is_err());
        }
    }

    #[test]
    fn service_connect_sends_only_the_name_for_daemon_resolution() {
        assert_eq!(
            request(&["service", "connect", "web", "--local-port", "18087"]),
            (
                "service.connect".into(),
                json!({"name":"web","local":"127.0.0.1:18087","transport":"quic"})
            )
        );
        // The old peer/target form is gone; the destination comes from the registry.
        assert!(Cli::try_parse_from([
            "werewolfctl",
            "service",
            "connect",
            "wolf-a",
            "127.0.0.1:8080",
            "--local-port",
            "18087"
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "werewolfctl",
            "service",
            "add",
            "web",
            "wolf-a",
            "127.0.0.1:8080",
            "ftp"
        ])
        .is_err());
    }
}
