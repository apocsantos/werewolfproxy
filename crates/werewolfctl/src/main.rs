mod governance;
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
    /// Offline governance files: keys, drafts, signatures, statements.
    Governance {
        #[command(subcommand)]
        command: governance::GovernanceCommands,
    },
    /// Reach a Pack member through another Pack member. A route grants nothing.
    Route {
        #[command(subcommand)]
        command: RouteCommands,
    },
}
#[derive(Subcommand)]
enum ProviderCommands {
    /// Append PEER as the next provider of NAME. It grants no access.
    Add {
        name: String,
        #[arg(required_unless_present = "from")]
        peer: Option<String>,
        /// Target on PEER, for example 127.0.0.1:8080.
        #[arg(required_unless_present = "from")]
        target: Option<String>,
        /// Take the target from NAME as Pack member PEER publishes it now.
        #[arg(long, value_name = "PEER", conflicts_with_all = ["peer", "target"])]
        from: Option<String>,
        /// Transport for --from (default quic).
        #[arg(long, requires = "from", value_parser = ["quic", "tcp"])]
        transport: Option<String>,
    },
    /// Remove PEER from NAME's providers; the next one moves up.
    Remove { name: String, peer: String },
}
#[derive(Subcommand)]
enum RouteCommands {
    /// Reach PEER through relay VIA; replaces PEER's existing route.
    Set {
        peer: String,
        #[arg(long)]
        via: String,
        #[arg(long, default_value = "quic", value_parser = ["quic", "tcp"])]
        hop: String,
    },
    List,
    Remove {
        peer: String,
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
        #[arg(required_unless_present = "from")]
        peer: Option<String>,
        /// Target on the peer, for example 127.0.0.1:8080.
        #[arg(required_unless_present = "from")]
        target: Option<String>,
        #[arg(required_unless_present = "from", value_parser = ["tcp", "http", "https", "ssh"])]
        kind: Option<String>,
        /// Copy NAME as Pack member PEER publishes it to this node, asked now.
        #[arg(long, value_name = "PEER", conflicts_with_all = ["peer", "target", "kind"])]
        from: Option<String>,
        /// Transport for --from (default quic).
        #[arg(long, requires = "from", value_parser = ["quic", "tcp"])]
        transport: Option<String>,
    },
    /// Offer a local target to Pack members already granted it. Grants nothing.
    Publish {
        name: String,
        /// Literal IP:port on this node, for example 127.0.0.1:8080.
        target: String,
        #[arg(value_parser = ["tcp", "http", "https", "ssh"])]
        kind: String,
    },
    Unpublish {
        name: String,
    },
    /// List published services and the Pack members each is visible to.
    Published,
    List,
    Show {
        name: String,
    },
    Remove {
        name: String,
    },
    /// Further providers of a service, tried in order per connection.
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
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
    /// Ask Pack members which services they publish to this node.
    Services {
        /// Ask only this Pack member.
        #[arg(long)]
        peer: Option<String>,
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
    let (command, args) = match cli.command {
        Commands::Governance { command } => match governance::run(cli.json, command)? {
            None => return Ok(()),
            Some(request) => request,
        },
        other => request_for(other)?,
    };
    let socket = socket_path(&cli.socket)?;
    let response = send_request(&socket, &command, args)
        .await
        .map_err(|e| CliError::new(ExitClass::DaemonUnavailable, e))?;
    print_response(cli.json, &command, response)
}

/// `--transport` only selects how `--from` asks the peer; without `--from`
/// it would be silently ignored, so it is refused instead.
fn transport_without_from() -> CliError {
    CliError::new(ExitClass::Usage, "--transport applies only with --from")
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
        Commands::Route {
            command: RouteCommands::Set { peer, via, hop },
        } => ("route.set", json!({"peer":peer,"via":via,"hop":hop})),
        Commands::Route {
            command: RouteCommands::List,
        } => ("route.list", json!({})),
        Commands::Route {
            command: RouteCommands::Remove { peer },
        } => ("route.remove", json!({"peer":peer})),
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
                ServiceCommands::Provider {
                    command:
                        ProviderCommands::Add {
                            name,
                            peer,
                            target,
                            from,
                            transport,
                        },
                },
        } => match from {
            Some(from) => {
                let _ = (peer, target);
                (
                    "service.provider.add_from",
                    json!({"name":name,"peer":from,"transport":transport.unwrap_or_else(|| "quic".into())}),
                )
            }
            None if transport.is_some() => return Err(transport_without_from()),
            None => (
                "service.provider.add",
                json!({"name":name,"peer":peer.unwrap_or_default(),"target":target.unwrap_or_default()}),
            ),
        },
        Commands::Service {
            command:
                ServiceCommands::Provider {
                    command: ProviderCommands::Remove { name, peer },
                },
        } => ("service.provider.remove", json!({"name":name,"peer":peer})),
        Commands::Service {
            command:
                ServiceCommands::Add {
                    name,
                    peer,
                    target,
                    kind,
                    from: Some(from),
                    transport,
                },
        } => {
            // The daemon asks the peer itself; the CLI never supplies the target.
            let _ = (peer, target, kind);
            (
                "service.add_from",
                json!({"name":name,"peer":from,"transport":transport.unwrap_or_else(|| "quic".into())}),
            )
        }
        Commands::Service {
            command:
                ServiceCommands::Add {
                    name,
                    peer,
                    target,
                    kind,
                    from: None,
                    transport,
                },
        } => (
            if transport.is_some() {
                return Err(transport_without_from());
            } else {
                "service.add"
            },
            json!({"name":name,"peer":peer.unwrap_or_default(),"target":target.unwrap_or_default(),"kind":kind.unwrap_or_default()}),
        ),
        Commands::Service {
            command: ServiceCommands::Publish { name, target, kind },
        } => (
            "service.publish",
            json!({"name":name,"target":target,"kind":kind}),
        ),
        Commands::Service {
            command: ServiceCommands::Unpublish { name },
        } => ("service.unpublish", json!({"name":name})),
        Commands::Service {
            command: ServiceCommands::Published,
        } => ("service.published", json!({})),
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
            DiscoverCommands::Services { peer, transport } => (
                "discover.services",
                json!({"peer":peer.unwrap_or_default(),"transport":transport}),
            ),
        },
        Commands::Version
        | Commands::Doctor
        | Commands::Governance { .. }
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
    fn stage25_service_commands_map_to_daemon_requests() {
        assert_eq!(
            request(&["service", "publish", "web", "127.0.0.1:8080", "http"]),
            (
                "service.publish".into(),
                json!({"name":"web","target":"127.0.0.1:8080","kind":"http"})
            )
        );
        assert_eq!(
            request(&["service", "unpublish", "web"]),
            ("service.unpublish".into(), json!({"name":"web"}))
        );
        assert_eq!(
            request(&["service", "published"]),
            ("service.published".into(), json!({}))
        );
        // --from sends only the name and peer; the daemon asks the peer.
        assert_eq!(
            request(&["service", "add", "web", "--from", "wolf-b"]),
            (
                "service.add_from".into(),
                json!({"name":"web","peer":"wolf-b","transport":"quic"})
            )
        );
        assert_eq!(
            request(&[
                "service",
                "add",
                "web",
                "--from",
                "wolf-b",
                "--transport",
                "tcp"
            ]),
            (
                "service.add_from".into(),
                json!({"name":"web","peer":"wolf-b","transport":"tcp"})
            )
        );
        assert_eq!(
            request(&["discover", "services"]),
            (
                "discover.services".into(),
                json!({"peer":"","transport":"quic"})
            )
        );
        assert_eq!(
            request(&[
                "discover",
                "services",
                "--peer",
                "wolf-b",
                "--transport",
                "tcp"
            ]),
            (
                "discover.services".into(),
                json!({"peer":"wolf-b","transport":"tcp"})
            )
        );
        for bad in [
            &["service", "add", "web", "--from", "wolf-b", "wolf-a"][..],
            &["service", "add", "web", "wolf-a", "127.0.0.1:8080"],
            &["service", "add", "web", "--transport", "tcp"],
            &["service", "publish", "web", "127.0.0.1:8080", "ftp"],
        ] {
            assert!(
                Cli::try_parse_from(std::iter::once("werewolfctl").chain(bad.iter().copied()))
                    .is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn provider_commands_map_to_daemon_requests() {
        assert_eq!(
            request(&[
                "service",
                "provider",
                "add",
                "web",
                "wolf-b",
                "127.0.0.1:8081"
            ]),
            (
                "service.provider.add".into(),
                json!({"name":"web","peer":"wolf-b","target":"127.0.0.1:8081"})
            )
        );
        assert_eq!(
            request(&["service", "provider", "add", "web", "--from", "wolf-b"]),
            (
                "service.provider.add_from".into(),
                json!({"name":"web","peer":"wolf-b","transport":"quic"})
            )
        );
        assert_eq!(
            request(&["service", "provider", "remove", "web", "wolf-b"]),
            (
                "service.provider.remove".into(),
                json!({"name":"web","peer":"wolf-b"})
            )
        );
        for bad in [
            &["service", "provider", "add", "web", "wolf-b"][..],
            &[
                "service", "provider", "add", "web", "--from", "wolf-b", "wolf-c",
            ],
            &["service", "provider", "remove", "web"],
        ] {
            assert!(
                Cli::try_parse_from(std::iter::once("werewolfctl").chain(bad.iter().copied()))
                    .is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn governance_file_commands_parse_and_stay_local() {
        let key = "A".repeat(43) + "=";
        let mut genesis = vec!["governance", "draft", "genesis", "--out", "g.draft"];
        for _ in 0..10 {
            genesis.extend(["--key", key.as_str()]);
        }
        for args in [
            genesis,
            vec!["governance", "keygen", "k.key"],
            vec![
                "governance",
                "draft",
                "rotate",
                "--after",
                "g.wwgov",
                "--set",
                "9=x",
                "--out",
                "r",
            ],
            vec![
                "governance",
                "draft",
                "revoke",
                "--after",
                "g.wwgov",
                "--member",
                "x",
                "--out",
                "r",
            ],
            vec![
                "governance",
                "sign",
                "d",
                "k.key",
                "--index",
                "3",
                "--out",
                "p",
            ],
            vec!["governance", "combine", "d", "p1", "p2", "--out", "s"],
            vec!["governance", "inspect", "s", "--chain", "g", "r"],
        ] {
            let cli =
                Cli::try_parse_from(std::iter::once("werewolfctl").chain(args.iter().copied()))
                    .unwrap_or_else(|e| panic!("{args:?}: {e}"));
            assert!(matches!(cli.command, Commands::Governance { .. }));
        }
        for bad in [
            &["governance", "sign", "d", "k.key", "--out", "p"][..],
            &["governance", "combine", "d", "--out", "s"],
            &[
                "governance",
                "draft",
                "revoke",
                "--after",
                "g",
                "--out",
                "r",
            ],
        ] {
            assert!(
                Cli::try_parse_from(std::iter::once("werewolfctl").chain(bad.iter().copied()))
                    .is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn transport_without_from_is_refused() {
        for args in [
            &[
                "service",
                "add",
                "web",
                "wolf-a",
                "127.0.0.1:1",
                "tcp",
                "--transport",
                "tcp",
            ][..],
            &[
                "service",
                "provider",
                "add",
                "web",
                "wolf-b",
                "127.0.0.1:1",
                "--transport",
                "tcp",
            ],
        ] {
            let cli =
                Cli::try_parse_from(std::iter::once("werewolfctl").chain(args.iter().copied()))
                    .unwrap();
            let error = request_for(cli.command).expect_err("accepted --transport without --from");
            assert!(matches!(error.class, ExitClass::Usage), "{args:?}");
        }
    }

    #[test]
    fn route_commands_map_to_daemon_requests() {
        assert_eq!(
            request(&["route", "set", "wolf-d", "--via", "wolf-b"]),
            (
                "route.set".into(),
                json!({"peer":"wolf-d","via":"wolf-b","hop":"quic"})
            )
        );
        assert_eq!(
            request(&["route", "set", "wolf-d", "--via", "wolf-b", "--hop", "tcp"]),
            (
                "route.set".into(),
                json!({"peer":"wolf-d","via":"wolf-b","hop":"tcp"})
            )
        );
        assert_eq!(
            request(&["route", "list"]),
            ("route.list".into(), json!({}))
        );
        assert_eq!(
            request(&["route", "remove", "wolf-d"]),
            ("route.remove".into(), json!({"peer":"wolf-d"}))
        );
        for bad in [
            &["route", "set", "wolf-d"][..],
            &[
                "route",
                "set",
                "wolf-d",
                "--via",
                "wolf-b",
                "--hop",
                "tcp-plain",
            ],
            &["route", "set", "wolf-d", "wolf-b"],
            &["route", "remove"],
        ] {
            assert!(
                Cli::try_parse_from(std::iter::once("werewolfctl").chain(bad.iter().copied()))
                    .is_err(),
                "{bad:?}"
            );
        }
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
