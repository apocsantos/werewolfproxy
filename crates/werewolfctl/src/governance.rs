//! `werewolfctl governance` file commands (Stage28 survey decision 2a).
//! They run locally and never contact a daemon: key holders use them on
//! offline machines.

use crate::{print_local, CliError, ExitClass};
use clap::Subcommand;
use serde_json::{json, Value};
use std::path::PathBuf;
use werewolf_core::governance::{check, tool, Body, Head, Key};

#[derive(Subcommand)]
pub(crate) enum GovernanceCommands {
    /// Apply a signed statement on this node (sent to the daemon). A genesis
    /// needs --expect-hash, its hash exchanged out of band.
    Apply {
        file: PathBuf,
        #[arg(long)]
        expect_hash: Option<String>,
    },
    /// This node's governance head (asked from the daemon).
    Status,
    /// Fetch and apply the statements after this node's head from its Pack
    /// members (sent to the daemon).
    Sync {
        #[arg(long)]
        peer: Option<String>,
        #[arg(long, default_value = "quic", value_parser = ["quic", "tcp"])]
        transport: String,
    },
    /// Stage31: turn automatic sync on or off on this node (sent to the
    /// daemon; saved in the Den, survives restarts).
    AutoSync {
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    /// Create one key holder's governance key file (mode 0600).
    Keygen { out: PathBuf },
    /// Create an unsigned statement for the key holders to sign.
    Draft {
        #[command(subcommand)]
        kind: DraftKind,
    },
    /// Sign a draft as governance key INDEX; writes a signature part.
    Sign {
        draft: PathBuf,
        key: PathBuf,
        #[arg(long)]
        index: u8,
        #[arg(long)]
        out: PathBuf,
    },
    /// Combine signature parts into the signed statement.
    Combine {
        draft: PathBuf,
        #[arg(required = true)]
        parts: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Show a statement and its hash; with --chain, verify it as a node would.
    Inspect {
        file: PathBuf,
        /// Earlier statements, genesis first.
        #[arg(long, num_args = 1..)]
        chain: Vec<PathBuf>,
        /// Expected genesis hash (hex); defaults to the genesis's own hash.
        #[arg(long)]
        expect_genesis: Option<String>,
    },
    /// Stage30 key check: a lower bound on how many governance keys are
    /// usable now. Offline; nothing in the network acts on it.
    Check {
        #[command(subcommand)]
        action: CheckAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum CheckAction {
    /// A fresh challenge for the network of the given chain.
    New {
        /// The chain, genesis first.
        #[arg(long, num_args = 1.., required = true)]
        chain: Vec<PathBuf>,
        /// Expected genesis hash (hex); defaults to the genesis's own hash.
        #[arg(long)]
        expect_genesis: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Answer a challenge with governance key INDEX.
    Sign {
        challenge: PathBuf,
        key: PathBuf,
        #[arg(long)]
        index: u8,
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify answers against the keys in force at the chain's head.
    Verify {
        challenge: PathBuf,
        #[arg(required = true)]
        answers: Vec<PathBuf>,
        /// The chain, genesis first.
        #[arg(long, num_args = 1.., required = true)]
        chain: Vec<PathBuf>,
        /// Expected genesis hash (hex); defaults to the genesis's own hash.
        #[arg(long)]
        expect_genesis: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum DraftKind {
    /// A new network: the ten governance public keys, in index order.
    Genesis {
        #[arg(long = "key", required = true)]
        keys: Vec<String>,
        /// Network ID (64 hex digits); random if omitted.
        #[arg(long)]
        network_id: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Replace governance keys: --set INDEX=PUBLIC_KEY_B64.
    Rotate {
        #[arg(long)]
        after: PathBuf,
        #[arg(long = "set", required = true)]
        set: Vec<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Admit members to the network roster by their Pelt public keys.
    Admit {
        #[arg(long)]
        after: PathBuf,
        #[arg(long = "member", required = true)]
        members: Vec<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Activate the network: one-way, from this statement on.
    Activate {
        #[arg(long)]
        after: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// The network's terminal decision (Stage30): nothing follows it.
    Death {
        #[arg(long)]
        after: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Revoke members network-wide by their Pelt public keys.
    Revoke {
        #[arg(long)]
        after: PathBuf,
        #[arg(long = "member", required = true)]
        members: Vec<String>,
        #[arg(long)]
        out: PathBuf,
    },
}

fn fail(error: impl std::fmt::Display) -> CliError {
    CliError::new(ExitClass::Rejected, error.to_string())
}

fn usage(message: &str) -> CliError {
    CliError::new(ExitClass::Usage, message)
}

fn keys(texts: &[String]) -> Result<Vec<Key>, CliError> {
    texts
        .iter()
        .map(|t| tool::public_key(t).map_err(fail))
        .collect()
}

/// Runs a local file command, or returns the control request for a daemon
/// command (`apply`, `status`).
pub(crate) fn run(
    json_output: bool,
    command: GovernanceCommands,
) -> Result<Option<(String, Value)>, CliError> {
    match command {
        GovernanceCommands::Apply { file, expect_hash } => {
            let statement = tool::read_statement(&file).map_err(fail)?;
            let mut args = json!({"statement": tool::encode_b64(&statement.encode())});
            if let Some(hash) = expect_hash {
                args["expect_hash"] = hash.into();
            }
            Ok(Some(("governance.apply".into(), args)))
        }
        GovernanceCommands::Status => Ok(Some(("governance.status".into(), json!({})))),
        GovernanceCommands::Sync { peer, transport } => Ok(Some((
            "governance.sync".into(),
            json!({"peer": peer.unwrap_or_default(), "transport": transport}),
        ))),
        GovernanceCommands::AutoSync { state } => Ok(Some((
            "governance.auto_sync".into(),
            json!({"state": state}),
        ))),
        local => run_local(json_output, local).map(|()| None),
    }
}

fn run_local(json_output: bool, command: GovernanceCommands) -> Result<(), CliError> {
    match command {
        GovernanceCommands::Apply { .. }
        | GovernanceCommands::Status
        | GovernanceCommands::Sync { .. }
        | GovernanceCommands::AutoSync { .. } => {
            unreachable!("daemon commands")
        }
        GovernanceCommands::Keygen { out } => {
            let public = tool::keygen(&out).map_err(fail)?;
            print_local(
                json_output,
                "governance.keygen",
                json!({"key_file": out, "public_key_b64": public}),
            )
        }
        GovernanceCommands::Draft { kind } => {
            let (draft, out) = match kind {
                DraftKind::Genesis {
                    keys: texts,
                    network_id,
                    out,
                } => {
                    if texts.len() != werewolf_core::governance::KEY_COUNT {
                        return Err(usage("genesis needs exactly 10 --key values"));
                    }
                    let network_id = network_id
                        .map(|id| tool::unhex::<32>(&id).map_err(fail))
                        .transpose()?;
                    (tool::draft_genesis(&keys(&texts)?, network_id), out)
                }
                DraftKind::Rotate { after, set, out } => {
                    let previous = tool::read_statement(&after).map_err(fail)?;
                    let entries = set
                        .iter()
                        .map(|entry| {
                            let (index, key) = entry
                                .split_once('=')
                                .ok_or_else(|| usage("--set takes INDEX=PUBLIC_KEY_B64"))?;
                            let index = index
                                .parse::<u8>()
                                .map_err(|_| usage("--set index must be 0 to 9"))?;
                            Ok((index, tool::public_key(key).map_err(fail)?))
                        })
                        .collect::<Result<Vec<_>, CliError>>()?;
                    (tool::draft_after(&previous, Body::Rotation(entries)), out)
                }
                DraftKind::Admit {
                    after,
                    members,
                    out,
                } => {
                    let previous = tool::read_statement(&after).map_err(fail)?;
                    (
                        tool::draft_after(&previous, Body::Admission(keys(&members)?)),
                        out,
                    )
                }
                DraftKind::Activate { after, out } => {
                    let previous = tool::read_statement(&after).map_err(fail)?;
                    (tool::draft_after(&previous, Body::Activation), out)
                }
                DraftKind::Death { after, out } => {
                    let previous = tool::read_statement(&after).map_err(fail)?;
                    (tool::draft_after(&previous, Body::Death), out)
                }
                DraftKind::Revoke {
                    after,
                    members,
                    out,
                } => {
                    let previous = tool::read_statement(&after).map_err(fail)?;
                    (
                        tool::draft_after(&previous, Body::Revocation(keys(&members)?)),
                        out,
                    )
                }
            };
            tool::write_statement(&out, &draft).map_err(fail)?;
            print_local(
                json_output,
                "governance.draft",
                json!({"draft": out, "statement": tool::describe(&draft)}),
            )
        }
        GovernanceCommands::Sign {
            draft,
            key,
            index,
            out,
        } => {
            let draft_statement = tool::read_statement(&draft).map_err(fail)?;
            let part = tool::sign_part(&draft_statement, &key, index).map_err(fail)?;
            tool::write_part(&out, &part).map_err(fail)?;
            print_local(
                json_output,
                "governance.sign",
                json!({"part": out, "index": index, "hash": part.hash}),
            )
        }
        GovernanceCommands::Combine { draft, parts, out } => {
            let draft = tool::read_statement(&draft).map_err(fail)?;
            let parts = parts
                .iter()
                .map(|p| tool::read_part(p).map_err(fail))
                .collect::<Result<Vec<_>, _>>()?;
            let statement = tool::combine(&draft, &parts).map_err(fail)?;
            tool::write_statement(&out, &statement).map_err(fail)?;
            print_local(
                json_output,
                "governance.combine",
                json!({"statement": out, "summary": tool::describe(&statement)}),
            )
        }
        GovernanceCommands::Inspect {
            file,
            chain,
            expect_genesis,
        } => {
            let statement = tool::read_statement(&file).map_err(fail)?;
            let mut result = json!({"statement": tool::describe(&statement)});
            if !chain.is_empty() || expect_genesis.is_some() {
                let mut statements = chain
                    .iter()
                    .map(|p| tool::read_statement(p).map_err(fail))
                    .collect::<Result<Vec<_>, _>>()?;
                statements.push(statement);
                let genesis_hash = match expect_genesis {
                    Some(hex) => tool::unhex::<32>(&hex).map_err(fail)?,
                    None => statements[0].core.hash(),
                };
                let head = tool::replay(&statements, &genesis_hash).map_err(fail)?;
                result["verified"] = json!({
                    "genesis_hash": tool::hex(&genesis_hash),
                    "head_sequence": head.sequence,
                    "head_hash": tool::hex(&head.hash),
                    "revoked": head.revoked.len(),
                    "dead_at": head.dead_at,
                });
            }
            print_local(json_output, "governance.inspect", result)
        }
        GovernanceCommands::Check { action } => run_check(json_output, action),
    }
}

/// Replays `chain` (genesis first) as a node would, pinned to
/// `expect_genesis` or the genesis's own hash.
fn chain_head(chain: &[PathBuf], expect_genesis: Option<String>) -> Result<Head, CliError> {
    let statements = chain
        .iter()
        .map(|p| tool::read_statement(p).map_err(fail))
        .collect::<Result<Vec<_>, _>>()?;
    let genesis_hash = match expect_genesis {
        Some(hex) => tool::unhex::<32>(&hex).map_err(fail)?,
        None => statements[0].core.hash(),
    };
    tool::replay(&statements, &genesis_hash).map_err(fail)
}

fn run_check(json_output: bool, action: CheckAction) -> Result<(), CliError> {
    match action {
        CheckAction::New {
            chain,
            expect_genesis,
            out,
        } => {
            let head = chain_head(&chain, expect_genesis)?;
            let challenge = check::Challenge::new(&head);
            check::write(&out, &challenge).map_err(fail)?;
            print_local(
                json_output,
                "governance.check.new",
                json!({"challenge": out, "network_id": challenge.network_id}),
            )
        }
        CheckAction::Sign {
            challenge,
            key,
            index,
            out,
        } => {
            let answer = check::sign_file(&challenge, &key, index).map_err(fail)?;
            check::write(&out, &answer).map_err(fail)?;
            print_local(
                json_output,
                "governance.check.sign",
                json!({"answer": out, "index": index}),
            )
        }
        CheckAction::Verify {
            challenge,
            answers,
            chain,
            expect_genesis,
        } => {
            let head = chain_head(&chain, expect_genesis)?;
            let challenge =
                check::Challenge::parse(&std::fs::read(&challenge).map_err(fail)?).map_err(fail)?;
            let answers = answers
                .iter()
                .map(|p| check::Answer::parse(&std::fs::read(p).map_err(fail)?).map_err(fail))
                .collect::<Result<Vec<_>, _>>()?;
            let (indices, verdict) = challenge.verify(&head, &answers).map_err(fail)?;
            print_local(
                json_output,
                "governance.check.verify",
                json!({
                    "valid_indices": indices,
                    "count": indices.len(),
                    "verdict": verdict.label(),
                }),
            )
        }
    }
}
