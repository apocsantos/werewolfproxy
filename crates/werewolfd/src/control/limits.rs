use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{Map, Value};
use std::{fmt, io, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{timeout, timeout_at, Instant},
};
use werewolf_core::protocol::{ControlRequest, ControlResponse};

pub(super) const CLIENTS: usize = 16;
pub(super) const REQUESTS: usize = 32;
pub(super) const LIFETIME: Duration = Duration::from_secs(60);
const REQUEST_BYTES: usize = 16 * 1024;
const RESPONSE_BYTES: usize = 1024 * 1024;
const IO_WINDOW: Duration = Duration::from_secs(5);
pub(super) const ADMISSION_WINDOW: Duration = Duration::from_secs(10);

fn rejected() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid control request")
}

pub(super) async fn read_request<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    end: Instant,
) -> io::Result<Option<Vec<u8>>> {
    read_before(reader, end.min(Instant::now() + IO_WINDOW)).await
}

async fn read_before<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    deadline: Instant,
) -> io::Result<Option<Vec<u8>>> {
    timeout_at(deadline, async {
        let mut line = Vec::new();
        loop {
            let bytes = reader.fill_buf().await?;
            if bytes.is_empty() {
                return if line.is_empty() {
                    Ok(None)
                } else {
                    Err(rejected())
                };
            }
            let count = bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |i| i + 1);
            if line.len() + count > REQUEST_BYTES {
                return Err(rejected());
            }
            line.extend_from_slice(&bytes[..count]);
            reader.consume(count);
            if line.last() == Some(&b'\n') {
                return Ok(Some(line));
            }
            if line.len() == REQUEST_BYTES {
                return Err(rejected());
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "control read timeout"))?
}

// Deserialize all objects without silently replacing duplicate keys, including
// nested args. A lexical depth bound is checked before recursive deserialization.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON value")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut m: M) -> Result<Unique, M::Error> {
                let mut out = Map::new();
                while let Some(key) = m.next_key::<String>()? {
                    if out.contains_key(&key) {
                        return Err(de::Error::custom("duplicate field"));
                    }
                    out.insert(key, m.next_value::<Unique>()?.0);
                }
                Ok(Unique(Value::Object(out)))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut s: S) -> Result<Unique, S::Error> {
                let mut out = Vec::new();
                while let Some(v) = s.next_element::<Unique>()? {
                    out.push(v.0);
                }
                Ok(Unique(Value::Array(out)))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Unique, E> {
                Ok(Unique(Value::String(s.into())))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Unique, E> {
                Ok(Unique(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|v| Unique(Value::Number(v)))
                    .ok_or_else(|| de::Error::custom("invalid number"))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
        }
        d.deserialize_any(V)
    }
}

pub(super) fn parse(line: &[u8]) -> io::Result<ControlRequest> {
    if line.len() > REQUEST_BYTES {
        return Err(rejected());
    }
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for b in line {
        if quoted {
            if escaped {
                escaped = false;
            } else if *b == b'\\' {
                escaped = true;
            } else if *b == b'"' {
                quoted = false;
            }
        } else {
            match b {
                b'"' => quoted = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > 16 {
                        return Err(rejected());
                    }
                }
                b'}' | b']' => {
                    depth = depth.checked_sub(1).ok_or_else(rejected)?;
                }
                _ => {}
            }
        }
    }
    let Unique(value) = serde_json::from_slice(line).map_err(|_| rejected())?;
    let object = value.as_object().ok_or_else(rejected)?;
    if object
        .keys()
        .any(|k| !matches!(k.as_str(), "id" | "cmd" | "args"))
    {
        return Err(rejected());
    }
    if object.get("args").is_some_and(|v| !v.is_object()) {
        return Err(rejected());
    }
    let mut request: ControlRequest = serde_json::from_value(value).map_err(|_| rejected())?;
    if request.id.len() > 128
        || request.cmd.is_empty()
        || request.cmd.len() > 64
        || !request.cmd.is_ascii()
    {
        return Err(rejected());
    }
    if request.args.is_null() {
        request.args = Value::Object(Map::new());
    }
    let args = request.args.as_object().ok_or_else(rejected)?;
    let allowed: &[&str] = match request.cmd.as_str() {
        "pack.add" => &["name", "fingerprint", "address"],
        "pack.set_address" => &["name", "address"],
        "pack.remove" | "pack.revoke" | "fang.profile.remove" | "fang.open_profile" => &["name"],
        "fang.open" => &["peer", "local", "remote", "transport"],
        "fang.profile.add" => &["name", "peer", "local", "remote", "transport"],
        "fang.close" => &["fang_id"],
        _ => &[],
    };
    for (key, value) in args {
        if !allowed.contains(&key.as_str()) {
            return Err(rejected());
        }
        let value = value.as_str().ok_or_else(rejected)?;
        let limit = match key.as_str() {
            "name" | "peer" => 128,
            "local" | "remote" | "address" => 512,
            _ => 128,
        };
        if value.len() > limit {
            return Err(rejected());
        }
    }
    Ok(request)
}

struct Capped(Vec<u8>);
impl io::Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len() + bytes.len() >= RESPONSE_BYTES {
            return Err(rejected());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    response: &ControlResponse,
    end: Instant,
) -> io::Result<()> {
    let mut out = Capped(Vec::new());
    serde_json::to_writer(&mut out, response).map_err(|_| rejected())?;
    out.0.push(b'\n');
    timeout_at(
        end.min(Instant::now() + IO_WINDOW),
        writer.write_all(&out.0),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "control write timeout"))?
}

pub(super) struct Mutations {
    active: Arc<Semaphore>,
    pending: Semaphore,
}
impl Default for Mutations {
    fn default() -> Self {
        Self {
            active: Arc::new(Semaphore::new(1)),
            pending: Semaphore::new(8),
        }
    }
}
impl Mutations {
    pub(super) async fn admit(&self) -> io::Result<OwnedSemaphorePermit> {
        if let Ok(permit) = self.active.clone().try_acquire_owned() {
            return Ok(permit);
        }
        let _waiting = self.pending.try_acquire().map_err(|_| rejected())?;
        timeout(ADMISSION_WINDOW, self.active.clone().acquire_owned())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "control admission timeout"))?
            .map_err(|_| rejected())
    }
}

pub(super) fn mutates(cmd: &str) -> bool {
    matches!(
        cmd,
        "pelt.init"
            | "pack.add"
            | "pack.set_address"
            | "pack.revoke"
            | "pack.remove"
            | "fang.open"
            | "fang.profile.add"
            | "fang.profile.remove"
            | "fang.open_profile"
            | "fang.close"
            | "silver.trigger"
            | "silver.reset"
            | "fang.cleanup"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, BufReader};

    #[test]
    fn strict_parser_and_bounds() {
        assert!(parse(br#"{"id":"1","cmd":"status","args":{}}"#).is_ok());
        for bad in [
            br#"{"id":"1","id":"2","cmd":"status"}"#.as_slice(),
            br#"{"id":"1","cmd":"status","extra":1}"#,
            br#"{"id":"1","cmd":"pack.add","args":{"name":"a","name":"b"}}"#,
            br#"{"id":"1","cmd":"status"} {}"#,
            br#"{"id":1,"cmd":"status"}"#,
            &[0xff],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert!(parse(&[b'['; 17]).is_err());
        assert!(parse(&vec![b' '; REQUEST_BYTES + 1]).is_err());
        let request =
            serde_json::json!({"id":"1","cmd":"pack.add","args":{"name":"x".repeat(129)}});
        assert!(parse(request.to_string().as_bytes()).is_err());
    }

    #[tokio::test]
    async fn fragmented_and_pipelined_lines_preserved() {
        let (mut sender, receiver) = duplex(128);
        let sending = tokio::spawn(async move {
            sender.write_all(b"fir").await.unwrap();
            tokio::task::yield_now().await;
            sender.write_all(b"st\nsecond\n").await.unwrap();
        });
        let mut reader = BufReader::new(receiver);
        let end = Instant::now() + IO_WINDOW;
        assert_eq!(
            read_request(&mut reader, end).await.unwrap().unwrap(),
            b"first\n"
        );
        assert_eq!(
            read_request(&mut reader, end).await.unwrap().unwrap(),
            b"second\n"
        );
        sending.await.unwrap();
    }

    #[tokio::test]
    async fn incomplete_oversized_and_slow_reader_bounded() {
        let (_sender, receiver) = duplex(16);
        let mut reader = BufReader::new(receiver);
        assert!(
            read_before(&mut reader, Instant::now() + Duration::from_millis(20))
                .await
                .is_err()
        );
        let oversized = vec![b'x'; REQUEST_BYTES + 1];
        let mut reader = BufReader::new(&oversized[..]);
        assert!(read_request(&mut reader, Instant::now() + IO_WINDOW)
            .await
            .is_err());
        let (mut writer, _reader) = duplex(1);
        let response = ControlResponse::ok("1", Value::String("data".into()));
        assert!(write_response(
            &mut writer,
            &response,
            Instant::now() + Duration::from_millis(20)
        )
        .await
        .is_err());
        let response = ControlResponse::ok("1", Value::String("x".repeat(RESPONSE_BYTES)));
        assert!(
            write_response(&mut writer, &response, Instant::now() + IO_WINDOW)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn mutation_pending_limit_and_permit_release() {
        let admissions = Mutations::default();
        let active = admissions.admit().await.unwrap();
        let pending: Vec<_> = (0..8)
            .map(|_| admissions.pending.try_acquire().unwrap())
            .collect();
        assert!(admissions.admit().await.is_err());
        drop(pending);
        drop(active);
        assert!(admissions.admit().await.is_ok());
    }
}
