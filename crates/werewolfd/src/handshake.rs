//! Versioned, bounded handshake messages. This module does not connect targets.
#![allow(dead_code)] // Primitives land before their transport callers.
use base64::{engine::general_purpose::STANDARD, Engine};
use rand_core::{CryptoRng, OsRng, RngCore};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{io, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::{timeout_at, Instant},
};
use werewolf_core::pelt::{
    fingerprint_from_public_key_b64, sign_message, verify_message, PeltIdentity,
};

pub(super) const TCP: &str = "fang-tcp-v3";
pub(super) const QUIC: &str = "fang-quic-v3";
pub(super) const SESSION: &str = "fang-v3-secure";
pub(super) const MESSAGE_LIMIT: usize = 4096;
pub(super) const CHALLENGE_LIMIT: usize = 512;
pub(super) const READ_WINDOW: Duration = Duration::from_secs(5);
pub(super) const WRITE_WINDOW: Duration = Duration::from_secs(1);
pub(super) const SERVER_WINDOW: Duration = Duration::from_secs(11);
pub(super) const CLIENT_WINDOW: Duration = Duration::from_secs(15);
// Repository/history audit found only orderly code 0 in tests, no production
// application code assignment. Reserve this value for all v3 handshake rejection.
pub(super) const V3_REJECT_CODE: u32 = 0x575703;

pub(super) fn rejected() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "handshake rejected")
}
pub(super) fn require(value: bool) -> io::Result<()> {
    if value {
        Ok(())
    } else {
        Err(rejected())
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn unhex<const N: usize>(text: &str) -> io::Result<[u8; N]> {
    require(text.len() == 2 * N)?;
    let mut result = [0; N];
    for (i, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        let digit = |b| match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            _ => Err(rejected()),
        };
        result[i] = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Ok(result)
}
pub(super) fn binary<const N: usize>(text: &str) -> io::Result<[u8; N]> {
    require(text.len() == N.div_ceil(3) * 4)?;
    let value: [u8; N] = STANDARD
        .decode(text)
        .map_err(|_| rejected())?
        .try_into()
        .map_err(|_| rejected())?;
    require(STANDARD.encode(value) == text)?;
    Ok(value)
}
pub(super) fn fingerprint(text: &str) -> io::Result<[u8; 8]> {
    let suffix = text.strip_prefix("wwp1:").ok_or_else(rejected)?;
    require(suffix.len() == 23)?;
    let mut result = [0; 8];
    for (i, part) in suffix.as_bytes().chunks(3).enumerate() {
        let digit = |b| match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            _ => Err(rejected()),
        };
        require(part.len() == if i == 7 { 2 } else { 3 })?;
        if i != 7 {
            require(part[2] == b'-')?;
        }
        result[i] = digit(part[0])? * 16 + digit(part[1])?;
    }
    Ok(result)
}
fn remote(text: &str) -> io::Result<()> {
    require(
        !text.is_empty()
            && text.len() <= 512
            && !text.chars().any(|c| c.is_whitespace() || c.is_control()),
    )
}
pub(super) fn random<const N: usize>() -> io::Result<[u8; N]> {
    random_with(&mut OsRng)
}
fn random_with<const N: usize, R: RngCore + CryptoRng>(rng: &mut R) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    rng.try_fill_bytes(&mut bytes).map_err(|_| rejected())?;
    Ok(bytes)
}
pub(super) fn transcript(domain: &str, fields: &[&[u8]]) -> Vec<u8> {
    let mut out = b"WWP-HS\0".to_vec();
    for field in std::iter::once(domain.as_bytes()).chain(fields.iter().copied()) {
        out.extend_from_slice(
            &u32::try_from(field.len())
                .expect("bounded transcript field")
                .to_be_bytes(),
        );
        out.extend_from_slice(field);
    }
    out
}
pub(super) fn verify(public: &str, message: &[u8], signature: &str) -> io::Result<()> {
    binary::<32>(public)?;
    binary::<64>(signature)?;
    verify_message(public, message, signature).map_err(|_| rejected())
}
pub(super) fn sign(identity: &PeltIdentity, message: &[u8]) -> io::Result<String> {
    sign_message(identity, message).map_err(|_| rejected())
}
fn public_matches(public: &str, fp: &str) -> io::Result<[u8; 32]> {
    let bytes = binary(public)?;
    require(fingerprint_from_public_key_b64(public).map_err(|_| rejected())? == fp)?;
    Ok(bytes)
}
pub(super) async fn read<T: DeserializeOwned, R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
    deadline: Instant,
) -> io::Result<T> {
    timeout_at(deadline, async {
        let mut data = Vec::with_capacity(limit);
        // Deliberately do not prefetch application bytes beyond LF.
        for _ in 0..limit {
            let byte = reader.read_u8().await?;
            data.push(byte);
            if byte == b'\n' {
                return serde_json::from_slice(&data).map_err(|_| rejected());
            }
        }
        Err(rejected())
    })
    .await
    .map_err(|_| rejected())?
}
pub(super) async fn write<T: Serialize, W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: &T,
    limit: usize,
    deadline: Instant,
) -> io::Result<()> {
    let mut data = serde_json::to_vec(value).map_err(|_| rejected())?;
    data.push(b'\n');
    require(data.len() <= limit)?;
    timeout_at(
        deadline.min(Instant::now() + WRITE_WINDOW),
        writer.write_all(&data),
    )
    .await
    .map_err(|_| rejected())?
}

pub(super) fn quic_binding(connection: &quinn::Connection) -> io::Result<[u8; 32]> {
    let mut exporter = [0; 32];
    connection
        .export_keying_material(
            &mut exporter,
            b"EXPORTER-WerewolfProxy-Fang-QUIC-v3",
            b"werewolfproxy/fang-quic-v3",
        )
        .map_err(|_| rejected())?;
    let mut hash = blake3::Hasher::new();
    hash.update(b"werewolfproxy/fang-quic-v3/channel-binding\0");
    hash.update(&exporter);
    // B is public channel-binding material, NOT a secret or bearer credential.
    // E is never transmitted, logged, persisted, or used by the TCP session KDF.
    Ok(*hash.finalize().as_bytes())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TcpChallenge {
    pub(super) cmd: String,
    pub(super) protocol: String,
    pub(super) receiver_fingerprint: String,
    pub(super) challenge: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TcpOpen {
    pub(super) cmd: String,
    pub(super) protocol: String,
    pub(super) sender_pubkey: String,
    pub(super) sender_fingerprint: String,
    pub(super) receiver_fingerprint: String,
    pub(super) challenge: String,
    pub(super) nonce: String,
    pub(super) client_x25519: String,
    pub(super) remote: String,
    pub(super) signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TcpAck {
    pub(super) cmd: String,
    pub(super) ok: bool,
    pub(super) protocol: String,
    pub(super) session: String,
    pub(super) receiver_pubkey: String,
    pub(super) receiver_fingerprint: String,
    pub(super) sender_fingerprint: String,
    pub(super) challenge: String,
    pub(super) nonce: String,
    pub(super) client_x25519: String,
    pub(super) server_x25519: String,
    pub(super) remote: String,
    pub(super) signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QuicOpen {
    pub(super) cmd: String,
    pub(super) protocol: String,
    pub(super) sender_fingerprint: String,
    pub(super) receiver_fingerprint: String,
    pub(super) nonce: String,
    pub(super) remote: String,
    pub(super) signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QuicAck {
    pub(super) cmd: String,
    pub(super) ok: bool,
    pub(super) protocol: String,
    pub(super) receiver_pubkey: String,
    pub(super) receiver_fingerprint: String,
    pub(super) sender_fingerprint: String,
    pub(super) nonce: String,
    pub(super) remote: String,
    pub(super) signature: String,
}

pub(super) struct TcpChallengeContext {
    pub(super) message: TcpChallenge,
    pub(super) receiver: PeltIdentity,
    pub(super) deadline: Instant,
    consumed: bool,
}
impl TcpChallengeContext {
    pub(super) fn new(receiver: PeltIdentity) -> io::Result<Self> {
        let message = TcpChallenge {
            cmd: "fang.tcp.challenge".into(),
            protocol: TCP.into(),
            receiver_fingerprint: receiver.fingerprint.clone(),
            challenge: hex(&random::<32>()?),
        };
        message.validate()?;
        public_matches(&receiver.public_key_b64, &receiver.fingerprint)?;
        Ok(Self {
            message,
            receiver,
            deadline: Instant::now() + READ_WINDOW,
            consumed: false,
        })
    }
    pub(super) fn check(&self, open: &TcpOpen) -> io::Result<()> {
        require(
            !self.consumed
                && Instant::now() < self.deadline
                && open.challenge == self.message.challenge
                && open.receiver_fingerprint == self.receiver.fingerprint,
        )
    }
    pub(super) fn consume(&mut self, open: &TcpOpen) -> io::Result<()> {
        self.check(open)?;
        self.consumed = true;
        Ok(())
    }
}
impl TcpChallenge {
    pub(super) fn validate(&self) -> io::Result<()> {
        require(self.cmd == "fang.tcp.challenge" && self.protocol == TCP)?;
        fingerprint(&self.receiver_fingerprint)?;
        unhex::<32>(&self.challenge)?;
        Ok(())
    }
}
impl TcpOpen {
    pub(super) fn transcript(&self) -> io::Result<Vec<u8>> {
        require(self.cmd == "fang.pipe" && self.protocol == TCP)?;
        fingerprint(&self.sender_fingerprint)?;
        fingerprint(&self.receiver_fingerprint)?;
        remote(&self.remote)?;
        let sp = public_matches(&self.sender_pubkey, &self.sender_fingerprint)?;
        Ok(transcript(
            "fang.tcp.open/v3",
            &[
                TCP.as_bytes(),
                self.sender_fingerprint.as_bytes(),
                &sp,
                self.receiver_fingerprint.as_bytes(),
                &unhex::<32>(&self.challenge)?,
                &unhex::<16>(&self.nonce)?,
                &binary::<32>(&self.client_x25519)?,
                self.remote.as_bytes(),
            ],
        ))
    }
    pub(super) fn new(
        sender: &PeltIdentity,
        challenge: &TcpChallenge,
        remote: &str,
        client_key: &[u8; 32],
    ) -> io::Result<Self> {
        challenge.validate()?;
        let mut value = Self {
            cmd: "fang.pipe".into(),
            protocol: TCP.into(),
            sender_pubkey: sender.public_key_b64.clone(),
            sender_fingerprint: sender.fingerprint.clone(),
            receiver_fingerprint: challenge.receiver_fingerprint.clone(),
            challenge: challenge.challenge.clone(),
            nonce: hex(&random::<16>()?),
            client_x25519: STANDARD.encode(client_key),
            remote: remote.into(),
            signature: String::new(),
        };
        value.signature = sign(sender, &value.transcript()?)?;
        Ok(value)
    }
}
impl TcpAck {
    pub(super) fn transcript(&self, open: &TcpOpen, retained: &[u8]) -> io::Result<Vec<u8>> {
        require(
            self.cmd == "fang.tcp.ack"
                && self.protocol == TCP
                && self.session == SESSION
                && self.ok
                && self.receiver_fingerprint == open.receiver_fingerprint
                && self.sender_fingerprint == open.sender_fingerprint
                && self.challenge == open.challenge
                && self.nonce == open.nonce
                && self.remote == open.remote
                && self.client_x25519 == open.client_x25519,
        )?;
        let rp = public_matches(&self.receiver_pubkey, &self.receiver_fingerprint)?;
        Ok(transcript(
            "fang.tcp.ack/v3",
            &[
                TCP.as_bytes(),
                &[1],
                SESSION.as_bytes(),
                retained,
                &rp,
                &binary::<32>(&self.server_x25519)?,
            ],
        ))
    }
    pub(super) fn new(
        open: &TcpOpen,
        retained: &[u8],
        receiver: &PeltIdentity,
        server_key: &[u8; 32],
    ) -> io::Result<Self> {
        let mut value = Self {
            cmd: "fang.tcp.ack".into(),
            ok: true,
            protocol: TCP.into(),
            session: SESSION.into(),
            receiver_pubkey: receiver.public_key_b64.clone(),
            receiver_fingerprint: receiver.fingerprint.clone(),
            sender_fingerprint: open.sender_fingerprint.clone(),
            challenge: open.challenge.clone(),
            nonce: open.nonce.clone(),
            client_x25519: open.client_x25519.clone(),
            server_x25519: STANDARD.encode(server_key),
            remote: open.remote.clone(),
            signature: String::new(),
        };
        value.signature = sign(receiver, &value.transcript(open, retained)?)?;
        Ok(value)
    }
}
impl QuicOpen {
    pub(super) fn transcript(&self, binding: &[u8; 32]) -> io::Result<Vec<u8>> {
        require(self.cmd == "fang.quic.open" && self.protocol == QUIC)?;
        fingerprint(&self.sender_fingerprint)?;
        fingerprint(&self.receiver_fingerprint)?;
        remote(&self.remote)?;
        Ok(transcript(
            "fang.quic.open/v3",
            &[
                QUIC.as_bytes(),
                self.sender_fingerprint.as_bytes(),
                self.receiver_fingerprint.as_bytes(),
                &unhex::<16>(&self.nonce)?,
                self.remote.as_bytes(),
                binding,
            ],
        ))
    }
    pub(super) fn new(
        sender: &PeltIdentity,
        receiver: &str,
        remote: &str,
        binding: &[u8; 32],
    ) -> io::Result<Self> {
        let mut value = Self {
            cmd: "fang.quic.open".into(),
            protocol: QUIC.into(),
            sender_fingerprint: sender.fingerprint.clone(),
            receiver_fingerprint: receiver.into(),
            nonce: hex(&random::<16>()?),
            remote: remote.into(),
            signature: String::new(),
        };
        value.signature = sign(sender, &value.transcript(binding)?)?;
        Ok(value)
    }
}
impl QuicAck {
    pub(super) fn transcript(&self, open: &QuicOpen, retained: &[u8]) -> io::Result<Vec<u8>> {
        require(
            self.cmd == "fang.quic.ack"
                && self.protocol == QUIC
                && self.ok
                && self.receiver_fingerprint == open.receiver_fingerprint
                && self.sender_fingerprint == open.sender_fingerprint
                && self.nonce == open.nonce
                && self.remote == open.remote,
        )?;
        Ok(transcript(
            "fang.quic.ack/v3",
            &[
                QUIC.as_bytes(),
                &[1],
                retained,
                &public_matches(&self.receiver_pubkey, &self.receiver_fingerprint)?,
            ],
        ))
    }
    pub(super) fn new(
        open: &QuicOpen,
        retained: &[u8],
        receiver: &PeltIdentity,
    ) -> io::Result<Self> {
        let mut value = Self {
            cmd: "fang.quic.ack".into(),
            ok: true,
            protocol: QUIC.into(),
            receiver_pubkey: receiver.public_key_b64.clone(),
            receiver_fingerprint: receiver.fingerprint.clone(),
            sender_fingerprint: open.sender_fingerprint.clone(),
            nonce: open.nonce.clone(),
            remote: open.remote.clone(),
            signature: String::new(),
        };
        value.signature = sign(receiver, &value.transcript(open, retained)?)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use werewolf_core::pelt::generate_identity;

    #[test]
    fn fixed_transcript_vectors() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/handshake_v3_transcripts.json"
        ))
        .unwrap();
        let sf = b"wwp1:01-23-45-67-89-AB-CD-EF";
        let rf = b"wwp1:FE-DC-BA-98-76-54-32-10";
        let range = |start, end| (start..end).collect::<Vec<u8>>();
        let open = transcript(
            "fang.tcp.open/v3",
            &[
                TCP.as_bytes(),
                sf,
                &range(0, 32),
                rf,
                &range(64, 96),
                &range(0, 16),
                &range(96, 128),
                b"127.0.0.1:8080",
            ],
        );
        assert_eq!(hex(&open), data["tcp_open"].as_str().unwrap());
        assert_eq!(
            hex(&transcript(
                "fang.tcp.ack/v3",
                &[
                    TCP.as_bytes(),
                    &[1],
                    SESSION.as_bytes(),
                    &open,
                    &range(32, 64),
                    &range(128, 160)
                ]
            )),
            data["tcp_ack"].as_str().unwrap()
        );
        let open = transcript(
            "fang.quic.open/v3",
            &[
                QUIC.as_bytes(),
                sf,
                rf,
                &range(0, 16),
                b"127.0.0.1:8080",
                &range(160, 192),
            ],
        );
        assert_eq!(hex(&open), data["quic_open"].as_str().unwrap());
        assert_eq!(
            hex(&transcript(
                "fang.quic.ack/v3",
                &[QUIC.as_bytes(), &[1], &open, &range(32, 64)]
            )),
            data["quic_ack"].as_str().unwrap()
        );
        assert_ne!(
            transcript("a", &[b"x|y", b"z"]),
            transcript("a", &[b"x", b"y|z"])
        );
    }

    #[test]
    fn random_canonicality_and_rng_failure() {
        let a = random::<16>().unwrap();
        let b = random::<16>().unwrap();
        assert_ne!(a, b);
        assert_eq!(unhex::<16>(&hex(&a)).unwrap(), a);
        for bad in [
            "A".repeat(32),
            "a".repeat(31),
            "g".repeat(32),
            "0".repeat(34),
        ] {
            assert!(unhex::<16>(&bad).is_err());
        }
        assert!(binary::<32>(&STANDARD.encode([0; 31])).is_err());
        assert!(binary::<32>(&STANDARD.encode([0; 32]).trim_end_matches('=').to_string()).is_err());
        struct Failed;
        impl CryptoRng for Failed {}
        impl RngCore for Failed {
            fn next_u32(&mut self) -> u32 {
                panic!("unused")
            }
            fn next_u64(&mut self) -> u64 {
                panic!("unused")
            }
            fn fill_bytes(&mut self, _: &mut [u8]) {
                panic!("must use fallible RNG")
            }
            fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
                Err(std::num::NonZeroU32::new(rand_core::Error::CUSTOM_START)
                    .unwrap()
                    .into())
            }
        }
        assert!(random_with::<32, _>(&mut Failed).is_err());
    }

    #[test]
    fn tcp_context_is_frozen_expiring_and_one_shot() {
        let sender = generate_identity();
        let receiver = generate_identity();
        let mut ctx = TcpChallengeContext::new(receiver.clone()).unwrap();
        let next = TcpChallengeContext::new(receiver).unwrap();
        assert_ne!(ctx.message.challenge, next.message.challenge);
        let open = TcpOpen::new(&sender, &ctx.message, "127.0.0.1:1", &[1; 32]).unwrap();
        let new_identity = generate_identity();
        assert_ne!(ctx.receiver.fingerprint, new_identity.fingerprint);
        ctx.consume(&open).unwrap();
        assert!(ctx.consume(&open).is_err());
        let mut expired = next;
        expired.deadline = Instant::now() - Duration::from_secs(1);
        let open = TcpOpen::new(&sender, &expired.message, "127.0.0.1:1", &[1; 32]).unwrap();
        assert!(expired.consume(&open).is_err());
    }

    #[test]
    fn ack_signatures_bind_complete_open_and_connection() {
        let sender = generate_identity();
        let receiver = generate_identity();
        let context = TcpChallengeContext::new(receiver.clone()).unwrap();
        let open = TcpOpen::new(&sender, &context.message, "127.0.0.1:1", &[1; 32]).unwrap();
        let retained = open.transcript().unwrap();
        let ack = TcpAck::new(&open, &retained, &receiver, &[2; 32]).unwrap();
        verify(
            &ack.receiver_pubkey,
            &ack.transcript(&open, &retained).unwrap(),
            &ack.signature,
        )
        .unwrap();
        let other = TcpOpen::new(&sender, &context.message, "127.0.0.1:1", &[1; 32]).unwrap();
        assert!(ack
            .transcript(&other, &other.transcript().unwrap())
            .is_err());
        let mut changed = retained.clone();
        changed[0] ^= 1;
        assert!(verify(
            &ack.receiver_pubkey,
            &ack.transcript(&open, &changed).unwrap(),
            &ack.signature
        )
        .is_err());
        let open = QuicOpen::new(&sender, &receiver.fingerprint, "127.0.0.1:1", &[3; 32]).unwrap();
        let retained = open.transcript(&[3; 32]).unwrap();
        let ack = QuicAck::new(&open, &retained, &receiver).unwrap();
        verify(
            &ack.receiver_pubkey,
            &ack.transcript(&open, &retained).unwrap(),
            &ack.signature,
        )
        .unwrap();
        assert!(verify(
            &ack.receiver_pubkey,
            &ack.transcript(&open, &open.transcript(&[4; 32]).unwrap())
                .unwrap(),
            &ack.signature
        )
        .is_err());
    }

    #[tokio::test]
    async fn bounded_fragmented_reader_preserves_following_bytes() {
        let message = TcpChallengeContext::new(generate_identity())
            .unwrap()
            .message;
        let mut bytes = serde_json::to_vec(&message).unwrap();
        bytes.extend_from_slice(b"\napplication");
        let (mut writer, mut reader) = tokio::io::duplex(8192);
        let task = tokio::spawn(async move {
            for byte in bytes {
                writer.write_all(&[byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        let parsed: TcpChallenge = read(&mut reader, 512, Instant::now() + READ_WINDOW)
            .await
            .unwrap();
        parsed.validate().unwrap();
        let mut tail = Vec::new();
        reader.read_to_end(&mut tail).await.unwrap();
        assert_eq!(tail, b"application");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn strict_parser_and_timeout_matrix() {
        for data in [
            b"{\"cmd\":\"x\",\"cmd\":\"y\",\"protocol\":\"p\",\"receiver_fingerprint\":\"f\",\"challenge\":\"c\"}\n".to_vec(),
            b"{\"cmd\":\"x\",\"protocol\":\"p\",\"receiver_fingerprint\":\"f\",\"challenge\":\"c\",\"unknown\":1}\n".to_vec(),
            b"{}\n".to_vec(),b"[]\n".to_vec(),b"{} {}\n".to_vec(),vec![b'x';513],
        ] {
            let (mut writer,mut reader)=tokio::io::duplex(8192);writer.write_all(&data).await.unwrap();drop(writer);
            assert!(read::<TcpChallenge,_>(&mut reader,512,Instant::now()+READ_WINDOW).await.is_err());
        }
        let (_writer, mut reader) = tokio::io::duplex(16);
        assert!(read::<TcpChallenge, _>(
            &mut reader,
            512,
            Instant::now() + Duration::from_millis(10)
        )
        .await
        .is_err());
    }
}
