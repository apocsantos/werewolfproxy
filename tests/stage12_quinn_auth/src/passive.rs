//! Minimal observer for this harness's QUIC v1 client Initial packets.
//! Uses only public DCID/salt and existing ring primitives; no TLS key log.
use crate::{hex, Datagram};
use ring::{aead, hkdf};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const V1_SALT: &[u8] = &[
    0x38, 0x76, 0x2c, 0xf7, 0xf5, 0x59, 0x34, 0xb3, 0x4d, 0x17, 0x9a, 0xe6, 0xa4, 0xc8, 0x0c, 0xad,
    0xcc, 0xbb, 0x7f, 0x0a,
];

struct Length(usize);
impl hkdf::KeyType for Length {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand(secret: &hkdf::Prk, label: &[u8], length: usize) -> Vec<u8> {
    let mut info = (length as u16).to_be_bytes().to_vec();
    info.push((6 + label.len()) as u8);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label);
    info.push(0); // empty context
    let parts = [&info[..]];
    let okm = secret.expand(&parts, Length(length)).unwrap();
    let mut output = vec![0; length];
    okm.fill(&mut output).unwrap();
    output
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> &'a [u8] {
    let result = &bytes[*cursor..*cursor + length];
    *cursor += length;
    result
}

fn varint(bytes: &[u8], cursor: &mut usize) -> usize {
    let first = bytes[*cursor];
    let length = 1 << (first >> 6);
    let data = take(bytes, cursor, length);
    data.iter()
        .skip(1)
        .fold((first & 0x3f) as usize, |value, byte| {
            (value << 8) | *byte as usize
        })
}

fn u16_length(bytes: &[u8], cursor: &mut usize) -> usize {
    u16::from_be_bytes(take(bytes, cursor, 2).try_into().unwrap()) as usize
}

fn client_initial(bytes: &[u8], original_dcid: &[u8]) -> (Vec<u8>, Value) {
    assert_eq!(&bytes[1..5], &[0, 0, 0, 1], "observer is scoped to QUIC v1");
    let mut cursor = 5;
    let dcid_len = take(bytes, &mut cursor, 1)[0] as usize;
    let dcid = take(bytes, &mut cursor, dcid_len);
    let scid_len = take(bytes, &mut cursor, 1)[0] as usize;
    take(bytes, &mut cursor, scid_len);
    let token_len = varint(bytes, &mut cursor);
    take(bytes, &mut cursor, token_len);
    let protected_length = varint(bytes, &mut cursor);
    let pn_offset = cursor;
    let end = pn_offset + protected_length;
    // Initial secrets stay bound to the first client DCID even after the
    // destination CID switches to the one supplied by the server.
    let initial_secret = hkdf::Salt::new(hkdf::HKDF_SHA256, V1_SALT).extract(original_dcid);
    let client_secret = expand(&initial_secret, b"client in", 32);
    let secret = hkdf::Prk::new_less_safe(hkdf::HKDF_SHA256, &client_secret);
    let key = expand(&secret, b"quic key", 16);
    let mut iv = expand(&secret, b"quic iv", 12);
    let hp = expand(&secret, b"quic hp", 16);
    let protector = aead::quic::HeaderProtectionKey::new(&aead::quic::AES_128, &hp).unwrap();
    let mask = protector
        .new_mask(&bytes[pn_offset + 4..pn_offset + 20])
        .unwrap();
    let mut header = bytes[..pn_offset + 4].to_vec();
    header[0] ^= mask[0] & 0x0f;
    let pn_len = (header[0] & 3) as usize + 1;
    for i in 0..pn_len {
        header[pn_offset + i] ^= mask[i + 1];
    }
    header.truncate(pn_offset + pn_len);
    // This disposable connection sends only a handful of Initial packets, so
    // its full packet numbers fit in the transmitted packet number bytes.
    let pn = header[pn_offset..]
        .iter()
        .fold(0u64, |number, byte| (number << 8) | *byte as u64);
    for (iv_byte, pn_byte) in iv[4..].iter_mut().zip(pn.to_be_bytes()) {
        *iv_byte ^= pn_byte;
    }
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &key).unwrap());
    let mut ciphertext = bytes[pn_offset + pn_len..end].to_vec();
    let plaintext = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(iv.try_into().unwrap()),
            aead::Aad::from(&header),
            &mut ciphertext,
        )
        .expect("public Initial key must authenticate the recorded ciphertext")
        .to_vec();
    (
        plaintext,
        json!({"version": 1, "dcid_hex": hex(dcid), "original_dcid_hex": hex(original_dcid), "packet_number": pn,
        "aead_authentication": "PASS", "keys": "public_v1_salt_and_dcid_only"}),
    )
}

fn crypto_fragments(plaintext: &[u8], stream: &mut BTreeMap<usize, u8>) {
    let mut cursor = 0;
    while cursor < plaintext.len() {
        match varint(plaintext, &mut cursor) {
            0 | 1 => {} // PADDING or PING
            frame @ (2 | 3) => {
                // ACK(_ECN)
                varint(plaintext, &mut cursor); // largest acknowledged
                varint(plaintext, &mut cursor); // delay
                let ranges = varint(plaintext, &mut cursor);
                varint(plaintext, &mut cursor); // first range
                for _ in 0..ranges {
                    varint(plaintext, &mut cursor);
                    varint(plaintext, &mut cursor);
                }
                if frame == 3 {
                    for _ in 0..3 {
                        varint(plaintext, &mut cursor);
                    }
                }
            }
            6 => {
                let offset = varint(plaintext, &mut cursor);
                let length = varint(plaintext, &mut cursor);
                for (i, byte) in take(plaintext, &mut cursor, length).iter().enumerate() {
                    if let Some(previous) = stream.insert(offset + i, *byte) {
                        assert_eq!(previous, *byte);
                    }
                }
            }
            0x1c => break, // CONNECTION_CLOSE: no following crypto in our capture.
            frame => panic!("unexpected frame {frame} in harness client Initial"),
        }
    }
}

fn client_hello(bytes: &[u8]) -> Value {
    assert_eq!(bytes[0], 1);
    let length = bytes[1..4]
        .iter()
        .fold(0usize, |n, b| (n << 8) | *b as usize);
    let bytes = &bytes[..4 + length];
    let mut cursor = 4 + 2 + 32; // handshake header, legacy version, random
    let sid_len = take(bytes, &mut cursor, 1)[0] as usize;
    take(bytes, &mut cursor, sid_len);
    let suites_len = u16_length(bytes, &mut cursor);
    let suites = hex(take(bytes, &mut cursor, suites_len));
    let compression_len = take(bytes, &mut cursor, 1)[0] as usize;
    take(bytes, &mut cursor, compression_len);
    let extensions_len = u16_length(bytes, &mut cursor);
    assert_eq!(cursor + extensions_len, bytes.len());
    let mut extensions = Vec::new();
    let mut sni = None;
    let mut alpn = Vec::new();
    let mut versions = Vec::new();
    while cursor < bytes.len() {
        let extension = u16_length(bytes, &mut cursor);
        let length = u16_length(bytes, &mut cursor);
        let value = take(bytes, &mut cursor, length);
        extensions.push(extension);
        match extension {
            0 => {
                let mut offset = 2;
                assert_eq!(take(value, &mut offset, 1), &[0]);
                let length = u16_length(value, &mut offset);
                sni = Some(String::from_utf8(take(value, &mut offset, length).to_vec()).unwrap());
            }
            16 => {
                let mut offset = 2;
                while offset < value.len() {
                    let length = take(value, &mut offset, 1)[0] as usize;
                    alpn.push(hex(take(value, &mut offset, length)));
                }
            }
            43 => {
                versions = value[1..].chunks_exact(2).map(hex).collect();
            }
            _ => {}
        }
    }
    assert_eq!(versions, vec!["0304"]);
    json!({"sni": sni, "alpn_hex": alpn, "tls_supported_versions_hex": versions,
        "pre_shared_key_present": extensions.contains(&41), "early_data_present": extensions.contains(&42),
        "extension_types": extensions, "cipher_suites_hex": suites,
        "client_hello_hex": hex(bytes), "client_hello_bytes": bytes.len()})
}

pub fn inspect(capture: &[Datagram]) -> Value {
    let mut stream = BTreeMap::new();
    let mut packets = Vec::new();
    let mut original_dcid = None;
    for (index, packet) in capture.iter().enumerate() {
        if packet.direction != "client_to_server" || packet.bytes[0] & 0xf0 != 0xc0 {
            continue;
        }
        let dcid = original_dcid.get_or_insert_with(|| {
            let length = packet.bytes[5] as usize;
            packet.bytes[6..6 + length].to_vec()
        });
        let (plaintext, metadata) = client_initial(&packet.bytes, dcid);
        packets.push(json!({"datagram": index, "metadata": metadata}));
        crypto_fragments(&plaintext, &mut stream);
    }
    assert!(!packets.is_empty());
    let mut crypto = Vec::new();
    for offset in 0..stream.len() {
        crypto.push(
            *stream
                .get(&offset)
                .expect("no gaps in captured ClientHello"),
        );
    }
    json!({"result": "PASS", "scope": "QUIC v1 client Initial / ClientHello only",
        "uses_endpoint_secrets_or_keylog": false, "packets": packets,
        "client_hello": client_hello(&crypto)})
}
