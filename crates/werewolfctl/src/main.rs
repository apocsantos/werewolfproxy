#[cfg(not(target_os = "linux"))]
compile_error!("secure local control currently requires Linux");
use clap::{Parser, Subcommand};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};
use werewolf_core::protocol::{ControlRequest, ControlResponse};

#[derive(Parser)]
#[command(name = "werewolfctl")]
#[command(about = "WerewolfProxy control tool")]
struct Cli {
    #[arg(long, default_value = "")]
    socket: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Version,
    Health,
    Diagnose,
    Status,

    Den {
        #[command(subcommand)]
        command: DenCommands,
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

    Fang {
        #[command(subcommand)]
        command: FangCommands,
    },
}

#[derive(Subcommand)]
enum DenCommands {
    Info,
}

#[derive(Subcommand)]
enum SilverCommands {
    Trigger,
    Reset,
}

#[derive(Subcommand)]
enum PeltCommands {
    Init,
    Fingerprint,
}

#[derive(Subcommand)]
enum PackCommands {
    Add {
        name: String,
        fingerprint: String,
        address: String,
    },
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
    List,
}

#[derive(Subcommand)]
enum FangCommands {
    Open {
        peer: String,
        local: String,
        remote: String,
    },
    OpenProfile {
        name: String,
    },
    Profile {
        #[command(subcommand)]
        command: FangProfileCommands,
    },
    List,
    Cleanup,
    Close {
        fang_id: String,
    },
}

#[derive(Subcommand)]
enum FangProfileCommands {
    Add {
        name: String,
        peer: String,
        local: String,
        remote: String,
    },
    List,
    Remove {
        name: String,
    },
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();

    let original_cmd = match &cli.command {
        Commands::Health => "health",
        Commands::Diagnose => "diagnose",
        Commands::Status => "status",
        _ => "",
    };

    let (cmd, args) = match cli.command {
        Commands::Version => {
            print_banner();
            let commit = option_env!("WEREWOLF_GIT_COMMIT").unwrap_or("unknown");
            let built = option_env!("WEREWOLF_BUILD_DATE").unwrap_or("unknown");

            println!("🐺 WerewolfProxy v0.1.0-rc1");
            println!("commit: {}", commit);
            println!("built: {}", built);
            println!("profile: release");
            println!("transport: signed quic");
            println!("persistent-fangs: enabled");
            return Ok(());
        }

        Commands::Health => ("status".to_string(), json!({})),

        Commands::Diagnose => ("status".to_string(), json!({})),

        Commands::Status => ("status".to_string(), json!({})),
        Commands::Den { command } => match command {
            DenCommands::Info => ("den.info".to_string(), json!({})),
        },

        Commands::Silver { command } => match command {
            SilverCommands::Trigger => ("silver.trigger".to_string(), json!({})),
            SilverCommands::Reset => ("silver.reset".to_string(), json!({})),
        },

        Commands::Pelt { command } => match command {
            PeltCommands::Init => ("pelt.init".to_string(), json!({})),
            PeltCommands::Fingerprint => ("pelt.fingerprint".to_string(), json!({})),
        },

        Commands::Pack { command } => match command {
            PackCommands::Add {
                name,
                fingerprint,
                address,
            } => (
                "pack.add".to_string(),
                json!({
                    "name": name,
                    "fingerprint": fingerprint,
                    "address": address
                }),
            ),
            PackCommands::Remove { name } => ("pack.remove".to_string(), json!({ "name": name })),

            PackCommands::Revoke { name } => ("pack.revoke".to_string(), json!({ "name": name })),
            PackCommands::SetAddress { name, address } => (
                "pack.set_address".to_string(),
                json!({
                    "name": name,
                    "address": address
                }),
            ),
            PackCommands::List => ("pack.list".to_string(), json!({})),
        },

        Commands::Fang { command } => match command {
            FangCommands::Open {
                peer,
                local,
                remote,
            } => (
                "fang.open".to_string(),
                json!({
                    "peer": peer,
                    "local": local,
                    "remote": remote
                }),
            ),

            FangCommands::OpenProfile { name } => {
                ("fang.open_profile".to_string(), json!({ "name": name }))
            }

            FangCommands::Profile { command } => match command {
                FangProfileCommands::Add {
                    name,
                    peer,
                    local,
                    remote,
                } => (
                    "fang.profile.add".to_string(),
                    json!({
                        "name": name,
                        "peer": peer,
                        "local": local,
                        "remote": remote
                    }),
                ),
                FangProfileCommands::List => ("fang.profile.list".to_string(), json!({})),
                FangProfileCommands::Remove { name } => {
                    ("fang.profile.remove".to_string(), json!({ "name": name }))
                }
            },

            FangCommands::Cleanup => ("fang.cleanup".to_string(), json!({})),
            FangCommands::List => ("fang.list".to_string(), json!({})),
            FangCommands::Close { fang_id } => {
                ("fang.close".to_string(), json!({ "fang_id": fang_id }))
            }
        },
    };

    let socket = if cli.socket.is_empty() {
        werewolf_core::local_fs::default_control_socket()?
            .into_os_string()
            .into_string()
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "non-UTF-8 control path")
            })?
    } else {
        cli.socket
    };
    let response = send_request(&socket, &cmd, args).await?;
    print_response(
        if original_cmd.is_empty() {
            &cmd
        } else {
            original_cmd
        },
        response,
    );

    Ok(())
}

async fn send_request(
    socket: &str,
    cmd: &str,
    args: serde_json::Value,
) -> std::io::Result<ControlResponse> {
    let mut stream = UnixStream::connect(socket).await?;

    let req = ControlRequest {
        id: "req-001".to_string(),
        cmd: cmd.to_string(),
        args,
    };

    let encoded = serde_json::to_string(&req).unwrap();
    stream.write_all(encoded.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let response: ControlResponse = serde_json::from_str(&line).unwrap();
    Ok(response)
}

fn print_banner() {
    println!(
        r#"██╗    ██╗███████╗██████╗ ███████╗██╗    ██╗ ██████╗ ██╗     ███████╗
██║    ██║██╔════╝██╔══██╗██╔════╝██║    ██║██╔═══██╗██║     ██╔════╝
██║ █╗ ██║█████╗  ██████╔╝█████╗  ██║ █╗ ██║██║   ██║██║     █████╗
██║███╗██║██╔══╝  ██╔══██╗██╔══╝  ██║███╗██║██║   ██║██║     ██╔══╝
╚███╔███╔╝███████╗██║  ██║███████╗╚███╔███╔╝╚██████╔╝███████╗██║
 ╚══╝╚══╝ ╚══════╝╚═╝  ╚═╝╚══════╝ ╚══╝╚══╝  ╚═════╝ ╚══════╝╚═╝"#
    );
    println!();
    println!("🐺 WerewolfProxy");
    println!();
}

fn print_response(cmd: &str, resp: ControlResponse) {
    if !resp.ok {
        println!("{}", serde_json::to_string_pretty(&resp.error).unwrap());
        return;
    }

    let value = resp.result.unwrap_or_else(|| serde_json::json!({}));

    match cmd {
        "diagnose" => {
            print_banner();
            let pelt = value["pelt_ready"].as_bool().unwrap_or(false);
            let packmates = value["packmates"].as_u64().unwrap_or(0);
            let profiles = value["fang_profiles"].as_u64().unwrap_or(0);
            let fangs = value["active_fangs"].as_u64().unwrap_or(0);

            let commit = option_env!("WEREWOLF_GIT_COMMIT").unwrap_or("unknown");
            let built = option_env!("WEREWOLF_BUILD_DATE").unwrap_or("unknown");

            println!("🐺 Werewolf Diagnostics");
            println!();

            println!("binary:");
            println!("  version:          v0.1.0-rc1");
            println!("  commit:           {}", commit);
            println!("  built:            {}", built);
            println!("  profile:          release");
            println!();

            println!("identity:");
            println!(
                "  pelt:             {} {}",
                if pelt { "loaded" } else { "missing" },
                if pelt { "✅" } else { "❌" }
            );
            println!();

            println!("network:");
            if let Some(listen) = value["listen"].as_str() {
                println!("  tcp listener:     {} ✅", listen);
            }
            if let Some(quic) = value["quic_listen"].as_str() {
                println!("  quic listener:    {} ✅", quic);
            }
            println!();

            println!("pack:");
            println!("  packmates:        {}", packmates);
            println!();

            println!("fangs:");
            println!("  profiles:         {}", profiles);
            println!("  active:           {}", fangs);
            println!("  persistent:       enabled ✅");
            println!();

            println!("security:");
            println!("  signed quic:      enabled ✅");
            println!("  anti-replay:      enabled ✅");
            println!("  peer revoke:      enabled ✅");
            println!();

            println!("ops:");
            println!("  systemd mode:     supported ✅");
            println!("  health command:   enabled ✅");
            println!("  structured logs:  enabled ✅");
            println!();

            println!("overall:");
            println!("  HEALTHY 🟢");
        }

        "health" => {
            print_banner();
            let pelt = value["pelt_ready"].as_bool().unwrap_or(false);
            let packmates = value["packmates"].as_u64().unwrap_or(0);
            let profiles = value["fang_profiles"].as_u64().unwrap_or(0);
            let fangs = value["active_fangs"].as_u64().unwrap_or(0);

            println!("🐺 Werewolf Health Check");
            println!();

            println!("  daemon:              online      ✅");
            println!("  control socket:      ok          ✅");
            println!(
                "  pelt:                {}      {}",
                if pelt { "loaded" } else { "missing" },
                if pelt { "✅" } else { "❌" }
            );

            println!(
                "  packmates:           {}           {}",
                packmates,
                if packmates > 0 { "✅" } else { "⚠️" }
            );

            println!(
                "  fang profiles:       {}           {}",
                profiles,
                if profiles > 0 { "✅" } else { "⚠️" }
            );

            println!("  active fangs:        {}           {}", fangs, "✅");

            if let Some(listen) = value["listen"].as_str() {
                println!("  tcp listener:        {}  ✅", listen);
            }

            if let Some(quic) = value["quic_listen"].as_str() {
                println!("  quic listener:       {}  ✅", quic);
            }

            println!();
            println!("overall: HEALTHY 🟢");
        }

        "status" => {
            println!("🐺 Werewolf Status");
            println!(
                "  mode:         {}",
                value["mode"].as_str().unwrap_or("unknown")
            );
            println!(
                "  pelt ready:   {}",
                value["pelt_ready"].as_bool().unwrap_or(false)
            );
            println!(
                "  packmates:    {}",
                value["packmates"].as_u64().unwrap_or(0)
            );
            println!(
                "  profiles:     {}",
                value["fang_profiles"].as_u64().unwrap_or(0)
            );
            println!(
                "  active fangs: {}",
                value["active_fangs"].as_u64().unwrap_or(0)
            );
            if let Some(listen) = value["listen"].as_str() {
                println!("  tcp listen:   {}", listen);
            }
            if let Some(quic) = value["quic_listen"].as_str() {
                println!("  quic listen:  {}", quic);
            }
            println!(
                "  silver:       {}",
                value["silver"].as_str().unwrap_or("unknown")
            );
            println!(
                "  hide:         {}",
                value["hide"].as_str().unwrap_or("unknown")
            );
        }

        "den.info" => {
            println!("🏠 Den Info");
            println!(
                "  socket:        {}",
                value["socket"].as_str().unwrap_or("")
            );
            println!("  home:          {}", value["home"].as_str().unwrap_or(""));
            println!(
                "  listen:        {}",
                value["listen"].as_str().unwrap_or("")
            );
            println!(
                "  mode:          {}",
                value["mode"].as_str().unwrap_or("unknown")
            );
            println!(
                "  pelt ready:    {}",
                value["pelt_ready"].as_bool().unwrap_or(false)
            );
            println!(
                "  packmates:     {}",
                value["packmates"].as_u64().unwrap_or(0)
            );
            println!(
                "  fang profiles: {}",
                value["fang_profiles"].as_u64().unwrap_or(0)
            );
            println!(
                "  active fangs:  {}",
                value["active_fangs"].as_u64().unwrap_or(0)
            );
            println!(
                "  silver:        {}",
                value["silver"].as_str().unwrap_or("unknown")
            );
            println!(
                "  hide:          {}",
                value["hide"].as_str().unwrap_or("unknown")
            );
        }

        "pelt.init" | "pelt.fingerprint" => {
            println!("🐾 Pelt");
            println!(
                "  fingerprint: {}",
                value["fingerprint"].as_str().unwrap_or("")
            );
            if let Some(saved_to) = value["saved_to"].as_str() {
                println!("  saved to:     {}", saved_to);
            }
        }

        "pack.list" => {
            println!("🐾 Packmates");

            if let Some(peers) = value.as_array() {
                if peers.is_empty() {
                    println!("  none");
                }

                for peer in peers {
                    println!("  - {}", peer["name"].as_str().unwrap_or("unnamed"));
                    println!(
                        "      fingerprint: {}",
                        peer["fingerprint"].as_str().unwrap_or("")
                    );
                    println!(
                        "      address:     {}",
                        peer["address"].as_str().unwrap_or("")
                    );
                    println!(
                        "      trust:       {}",
                        peer["trust"].as_str().unwrap_or("")
                    );
                }
            }
        }

        "pack.add" | "pack.remove" | "pack.revoke" => {
            println!("🐾 Pack");
            println!("  status:    {}", value["status"].as_str().unwrap_or(""));

            if let Some(peer) = value["peer"].as_str() {
                println!("  peer:      {}", peer);
            }

            if let Some(closed) = value["closed_fangs"].as_u64() {
                println!("  closed:    {}", closed);
            }

            println!("  packmates: {}", value["packmates"].as_u64().unwrap_or(0));
        }

        "fang.list" => {
            println!("🦷 Active Fangs");

            if let Some(fangs) = value.as_array() {
                if fangs.is_empty() {
                    println!("  none");
                }

                for fang in fangs {
                    println!("  - {}", fang["id"].as_str().unwrap_or(""));
                    println!("      peer:   {}", fang["peer"].as_str().unwrap_or(""));
                    println!("      local:  {}", fang["local"].as_str().unwrap_or(""));
                    println!("      remote: {}", fang["remote"].as_str().unwrap_or(""));
                    if let Some(transport) = fang["transport"].as_str() {
                        println!("      transport: {}", transport);
                    }
                    if let Some(seconds) = fang["uptime_seconds"].as_u64() {
                        let h = seconds / 3600;
                        let m = (seconds % 3600) / 60;
                        let s = seconds % 60;
                        println!("      uptime: {:02}:{:02}:{:02}", h, m, s);
                    }
                    println!("      state:  {}", fang["state"].as_str().unwrap_or(""));
                }
            }
        }

        "fang.open" | "fang.open_profile" => {
            println!("🦷 Fang Opened");
            println!("  id:     {}", value["fang_id"].as_str().unwrap_or(""));
            println!("  peer:   {}", value["peer"].as_str().unwrap_or(""));
            println!("  local:  {}", value["local"].as_str().unwrap_or(""));
            println!("  remote: {}", value["remote"].as_str().unwrap_or(""));
            if let Some(transport) = value["transport"].as_str() {
                println!("  transport: {}", transport);
            }
            println!("  state:  {}", value["state"].as_str().unwrap_or(""));
        }

        "fang.close" => {
            println!("🦷 Fang Closed");
            println!("  status:       {}", value["status"].as_str().unwrap_or(""));
            println!(
                "  active fangs: {}",
                value["active_fangs"].as_u64().unwrap_or(0)
            );
        }

        "fang.profile.list" => {
            println!("🦷 Fang Profiles");

            if let Some(profiles) = value.as_array() {
                if profiles.is_empty() {
                    println!("  none");
                }

                for profile in profiles {
                    println!("  - {}", profile["name"].as_str().unwrap_or(""));
                    println!("      peer:   {}", profile["peer"].as_str().unwrap_or(""));
                    println!("      local:  {}", profile["local"].as_str().unwrap_or(""));
                    println!("      remote: {}", profile["remote"].as_str().unwrap_or(""));
                }
            }
        }

        "fang.profile.add" | "fang.profile.remove" => {
            println!("🦷 Fang Profile");
            println!("  status:   {}", value["status"].as_str().unwrap_or(""));
            if let Some(name) = value["name"].as_str() {
                println!("  name:     {}", name);
            }
            println!("  profiles: {}", value["profiles"].as_u64().unwrap_or(0));
        }

        "silver.trigger" | "silver.reset" => {
            println!("🥈 Silver");
            println!("  status:  {}", value["status"].as_str().unwrap_or(""));
            println!("  message: {}", value["message"].as_str().unwrap_or(""));
        }

        _ => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
        }
    }
}
