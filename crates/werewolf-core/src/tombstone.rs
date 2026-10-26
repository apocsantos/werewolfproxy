//! Stage30 Tombstone (`docs/STAGE30_LIFECYCLE_SPEC.md` section 5): the one
//! file left in a destroyed Den. Strict JSON, fields in a fixed order, no
//! field beyond the schema, at most 8 KiB. Parsing accepts exactly the
//! canonical bytes, so a Tombstone has one encoding.
//!
//! Deliberately absent: the node's Pelt, governance public keys, the
//! roster, revoked keys, the Pack, addresses, routes, services, grants,
//! earlier statements, timestamps and free text.

use crate::governance::{Body, Statement};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::io;

pub const FILE_NAME: &str = "tombstone.json";
pub const MAX_LEN: usize = 8 * 1024;
const FORMAT: &str = "werewolf-tombstone";
const MAX_SOFTWARE_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Death,
    Retirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destruction {
    Incomplete,
    Complete,
}

/// A validated Tombstone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    kind: Kind,
    /// The death statement, for `Kind::Death` only.
    death: Option<Statement>,
    pub destruction: Destruction,
    software: String,
}

/// The on-disk form, in the schema's field order.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    version: u8,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    network_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    death_statement: Option<String>,
    destruction: String,
    software: String,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid tombstone")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn check_software(software: &str) -> io::Result<()> {
    if software.is_empty()
        || software.len() > MAX_SOFTWARE_LEN
        || !software.bytes().all(|c| (0x20..=0x7e).contains(&c))
    {
        return Err(invalid());
    }
    Ok(())
}

impl Tombstone {
    /// The Tombstone of a network death; `statement` must be a death
    /// statement (kind 5).
    pub fn death(
        statement: Statement,
        destruction: Destruction,
        software: &str,
    ) -> io::Result<Tombstone> {
        if !matches!(statement.core.body, Body::Death) {
            return Err(invalid());
        }
        check_software(software)?;
        let tombstone = Tombstone {
            kind: Kind::Death,
            death: Some(statement),
            destruction,
            software: software.into(),
        };
        // The statement must fit the size bound once encoded.
        tombstone.encode()?;
        Ok(tombstone)
    }

    /// The Tombstone of a local retirement: no chain data at all.
    pub fn retirement(destruction: Destruction, software: &str) -> io::Result<Tombstone> {
        check_software(software)?;
        Ok(Tombstone {
            kind: Kind::Retirement,
            death: None,
            destruction,
            software: software.into(),
        })
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn death_statement(&self) -> Option<&Statement> {
        self.death.as_ref()
    }

    pub fn software(&self) -> &str {
        &self.software
    }

    fn document(&self) -> Document {
        let core = self.death.as_ref().map(|statement| &statement.core);
        Document {
            format: FORMAT.into(),
            version: 1,
            kind: match self.kind {
                Kind::Death => "death",
                Kind::Retirement => "retirement",
            }
            .into(),
            network_id: core.map(|core| hex(&core.network_id)),
            terminal_sequence: core.map(|core| core.sequence),
            terminal_hash: core.map(|core| hex(&core.hash())),
            death_statement: self.death.as_ref().map(|s| STANDARD.encode(s.encode())),
            destruction: match self.destruction {
                Destruction::Incomplete => "incomplete",
                Destruction::Complete => "complete",
            }
            .into(),
            software: self.software.clone(),
        }
    }

    /// The canonical bytes, at most `MAX_LEN`.
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let bytes = serde_json::to_vec(&self.document()).map_err(|_| invalid())?;
        if bytes.len() > MAX_LEN {
            return Err(invalid());
        }
        Ok(bytes)
    }

    /// Strict parse: exactly the canonical bytes of a consistent Tombstone.
    pub fn parse(bytes: &[u8]) -> io::Result<Tombstone> {
        if bytes.len() > MAX_LEN {
            return Err(invalid());
        }
        let doc: Document = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        if doc.format != FORMAT || doc.version != 1 {
            return Err(invalid());
        }
        let destruction = match doc.destruction.as_str() {
            "incomplete" => Destruction::Incomplete,
            "complete" => Destruction::Complete,
            _ => return Err(invalid()),
        };
        let tombstone = match doc.kind.as_str() {
            "death" => {
                let encoded = doc.death_statement.as_deref().ok_or_else(invalid)?;
                let statement = STANDARD
                    .decode(encoded)
                    .ok()
                    .and_then(|bytes| Statement::decode(&bytes).ok())
                    .ok_or_else(invalid)?;
                let tombstone = Tombstone::death(statement, destruction, &doc.software)?;
                // The fields must agree with the statement (spec section 5).
                let expected = tombstone.document();
                if doc.network_id != expected.network_id
                    || doc.terminal_sequence != expected.terminal_sequence
                    || doc.terminal_hash != expected.terminal_hash
                {
                    return Err(invalid());
                }
                tombstone
            }
            "retirement" => {
                if doc.network_id.is_some()
                    || doc.terminal_sequence.is_some()
                    || doc.terminal_hash.is_some()
                    || doc.death_statement.is_some()
                {
                    return Err(invalid());
                }
                Tombstone::retirement(destruction, &doc.software)?
            }
            _ => return Err(invalid()),
        };
        if tombstone.encode()? != bytes {
            return Err(invalid());
        }
        Ok(tombstone)
    }
}

#[cfg(test)]
mod tests;
