//! Dialling a peer address that may name several IP addresses (Stage46,
//! `docs/STAGE46_DIAL_FALLBACK_SURVEY.md`).
//!
//! A Pack address can be a hostname. Before Stage46 a QUIC dial took its
//! first resolved address only, and TCP tried the addresses one after
//! another, so a first address that dropped packets used the whole window.
//! Every dial of a peer now goes through [`race`] (S46-D2, R1, RFC 8305 §5
//! without its DNS part): the first address is tried at once; if it has not
//! connected after [`ATTEMPT_DELAY`], or as soon as it fails, the next one
//! starts too, alternating address families ([`interleave`]); the first to
//! connect wins and every other attempt is dropped. The caller's window
//! bounds the whole race, as it bounded the single dial before. A literal
//! address is one attempt, exactly as before.
//!
//! Identity never depends on the name: every attempt pins the peer's Pack
//! key, and the TLS server name is the IP literal dialled. Another address
//! of the same name can only reach the same peer or fail its handshake.

use crate::handshake as hs;
use std::{collections::HashMap, future::Future, io, net::SocketAddr, time::Duration};
use tokio::{net::TcpStream, task::JoinSet, time::Instant};

/// How long an attempt runs alone before the next address starts as well
/// (RFC 8305's recommended Connection Attempt Delay).
pub(crate) const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// Every address `address` (`HOST:PORT`) names, in resolver order,
/// interleaved by family. A literal is returned as is, without a lookup.
/// The lookup is bounded by `hs::READ_WINDOW`.
pub(crate) async fn resolve_all(address: &str) -> io::Result<Vec<SocketAddr>> {
    if let Ok(literal) = address.parse() {
        return Ok(vec![literal]);
    }
    let addresses: Vec<SocketAddr> =
        tokio::time::timeout(hs::READ_WINDOW, tokio::net::lookup_host(address))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "name resolution timed out"))??
            .collect();
    if addresses.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "name resolved to no address",
        ));
    }
    Ok(interleave(addresses))
}

/// RFC 8305 §4: the first address's family first, then alternating
/// families, each family in its original order; duplicates dropped.
pub(crate) fn interleave(addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let mut unique = Vec::with_capacity(addresses.len());
    for address in addresses {
        if !unique.contains(&address) {
            unique.push(address);
        }
    }
    let Some(first) = unique.first().copied() else {
        return unique;
    };
    let (mut lead, mut other): (Vec<_>, Vec<_>) = unique
        .into_iter()
        .partition(|address| address.is_ipv6() == first.is_ipv6());
    lead.reverse();
    other.reverse();
    let mut ordered = Vec::with_capacity(lead.len() + other.len());
    while !lead.is_empty() || !other.is_empty() {
        ordered.extend(lead.pop());
        ordered.extend(other.pop());
    }
    ordered
}

/// Runs `attempt` on `addresses` as a staggered race (module docs). Returns
/// the first success; when every attempt failed, the most telling error:
/// the first from a peer that was reached (its refusal), otherwise the first.
/// Unbounded in time: the caller's window bounds it. The attempts still
/// running when one wins are aborted, which drops them.
pub(crate) async fn race<T, F, Fut>(addresses: &[SocketAddr], attempt: F) -> io::Result<T>
where
    T: Send + 'static,
    F: Fn(SocketAddr) -> Fut,
    Fut: Future<Output = io::Result<T>> + Send + 'static,
{
    let mut pending = addresses.iter().copied();
    let Some(first) = pending.next() else {
        return Err(hs::local_dial_failed("no address to dial"));
    };
    let mut running = JoinSet::new();
    running.spawn(attempt(first));
    let mut next_at = Instant::now() + ATTEMPT_DELAY;
    let mut errors: Vec<io::Error> = Vec::new();
    let mut more = pending.len() > 0;
    loop {
        tokio::select! {
            joined = running.join_next() => {
                let result = match joined {
                    Some(Ok(result)) => result,
                    Some(Err(error)) => Err(io::Error::other(error)),
                    None => unreachable!("an attempt is always running here"),
                };
                match result {
                    Ok(value) => return Ok(value),
                    Err(error) => errors.push(error),
                }
                // A failed attempt starts the next address at once.
                if let Some(address) = pending.next() {
                    running.spawn(attempt(address));
                    next_at = Instant::now() + ATTEMPT_DELAY;
                } else if running.is_empty() {
                    return Err(most_telling(errors));
                }
                more = pending.len() > 0;
            }
            _ = tokio::time::sleep_until(next_at), if more => {
                if let Some(address) = pending.next() {
                    running.spawn(attempt(address));
                }
                next_at = Instant::now() + ATTEMPT_DELAY;
                more = pending.len() > 0;
            }
        }
    }
}

fn most_telling(errors: Vec<io::Error>) -> io::Error {
    let reached = errors
        .iter()
        .position(|error| !hs::no_connection_made(error));
    let index = reached.unwrap_or(0);
    errors
        .into_iter()
        .nth(index)
        .unwrap_or_else(|| hs::local_dial_failed("no address to dial"))
}

/// A TCP connection to `address` (`HOST:PORT`): every address it names,
/// raced (S46-D4, T1). Unbounded in time: callers keep their own window.
pub(crate) async fn connect_tcp(address: &str) -> io::Result<TcpStream> {
    let addresses = resolve_all(address).await?;
    race(&addresses, TcpStream::connect).await
}

/// Pack hostnames resolved before a Fang opens (S46-D1, A1). The open looks
/// up a QUIC peer's, relay's or provider's address here, so no lookup runs
/// while the daemon's state is locked. A literal needs no entry.
#[derive(Default)]
pub(crate) struct Resolved(HashMap<String, Result<Vec<SocketAddr>, String>>);

impl Resolved {
    /// Resolves every hostname among `addresses` (`HOST:PORT`), concurrently;
    /// each lookup is bounded by `hs::READ_WINDOW`.
    pub(crate) async fn names(addresses: Vec<String>) -> Self {
        let mut lookups = JoinSet::new();
        for address in addresses {
            if address.parse::<SocketAddr>().is_err() {
                lookups.spawn(async move {
                    let result = resolve_all(&address)
                        .await
                        .map_err(|error| error.to_string());
                    (address, result)
                });
            }
        }
        let mut resolved = HashMap::new();
        while let Some(joined) = lookups.join_next().await {
            if let Ok((address, result)) = joined {
                resolved.insert(address, result);
            }
        }
        Self(resolved)
    }

    /// `address`'s addresses: a literal as is; a hostname as resolved, or
    /// the reason it did not; `None` if it was not resolved before the open
    /// (the Pack changed meanwhile).
    pub(crate) fn get(&self, address: &str) -> Option<Result<Vec<SocketAddr>, String>> {
        if let Ok(literal) = address.parse() {
            return Some(Ok(vec![literal]));
        }
        self.0.get(address).cloned()
    }

    /// A table as `names` would build it, for tests.
    #[cfg(test)]
    pub(crate) fn from_entries(
        entries: impl IntoIterator<Item = (&'static str, Result<Vec<SocketAddr>, String>)>,
    ) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(name, result)| (name.to_string(), result))
                .collect(),
        )
    }

    /// As `get`, as the error an open reports.
    pub(crate) fn addresses(&self, address: &str) -> Result<Vec<SocketAddr>, String> {
        match self.get(address) {
            Some(Ok(addresses)) => Ok(addresses),
            Some(Err(reason)) => Err(format!("does not resolve: {reason}")),
            None => Err("changed while the Fang was opening; retry".into()),
        }
    }
}

#[cfg(test)]
mod tests;
