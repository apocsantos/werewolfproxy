use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "werewolfd")]
pub(super) struct Args {
    #[arg(long, default_value = "")]
    pub(super) socket: String,

    #[arg(long, default_value = "~/.config/werewolf")]
    pub(super) home: String,

    #[arg(long, default_value = "127.0.0.1:8443")]
    pub(super) listen: String,

    #[arg(long, default_value = "127.0.0.1:9560")]
    pub(super) quic_listen: String,
}
