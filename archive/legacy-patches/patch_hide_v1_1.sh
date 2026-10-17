#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

# Add RngCore import
s = s.replace(
    "use rand_core::OsRng;",
    "use rand_core::{OsRng, RngCore};"
)

old = '''    const PLAIN_FRAME_SIZE: usize = 2048;

    if plaintext.len() > PLAIN_FRAME_SIZE - 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plaintext frame exceeds hide limit",
        ));
    }

    let mut padded_plaintext = Vec::with_capacity(PLAIN_FRAME_SIZE);
    padded_plaintext.extend_from_slice(&(plaintext.len() as u16).to_be_bytes());
    padded_plaintext.extend_from_slice(plaintext);
    padded_plaintext.resize(PLAIN_FRAME_SIZE, 0);'''

new = '''    const MIN_FRAME_SIZE: usize = 768;
    const MAX_FRAME_SIZE: usize = 2048;
    const STEP: usize = 128;

    if plaintext.len() > MAX_FRAME_SIZE - 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plaintext frame exceeds hide limit",
        ));
    }

    let minimum_needed = plaintext.len() + 2;
    let min_bucket = minimum_needed.max(MIN_FRAME_SIZE);
    let buckets = ((MAX_FRAME_SIZE - min_bucket) / STEP) + 1;

    let random_bucket = if buckets > 1 {
        (OsRng.next_u32() as usize) % buckets
    } else {
        0
    };

    let frame_size = min_bucket + (random_bucket * STEP);

    let mut padded_plaintext = Vec::with_capacity(frame_size);
    padded_plaintext.extend_from_slice(&(plaintext.len() as u16).to_be_bytes());
    padded_plaintext.extend_from_slice(plaintext);
    padded_plaintext.resize(frame_size, 0);'''

if old not in s:
    raise SystemExit("Hide v1 frame block not found")

s = s.replace(old, new)

p.write_text(s)
PY

echo "🌫️ Hide v1.1 randomized padding patch applied."
