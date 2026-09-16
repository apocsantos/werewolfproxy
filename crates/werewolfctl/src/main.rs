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

const VERSION: &str = "v0.1.0-rc1";

#[derive(Parser)]
#[command(
    name = "werewolfctl",
    about = "WerewolfProxy local administration tool"
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
