use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    pub fn from_name_address(name: &str, address: &str) -> Self {
        let mut h = Sha256::new();

        h.update(name.as_bytes());
        h.update(b"|");
        h.update(address.as_bytes());

        let digest = h.finalize();

        let mut id = [0u8; 32];
        id.copy_from_slice(&digest);

        Self(id)
    }

    /// Domain tag for Pelt-derived node IDs (Stage24 design).
    pub const DOMAIN: &'static [u8] = b"werewolf/node-id/v1\0";

    /// The overlay ID of the node holding this Pelt Ed25519 public key. It is
    /// stable across address changes and cannot be claimed without the key.
    pub fn from_public_key(public_key: &[u8; 32]) -> Self {
        let mut h = Sha256::new();
        h.update(Self::DOMAIN);
        h.update(public_key);
        Self(h.finalize().into())
    }

    pub fn xor_distance(&self, other: &NodeId) -> [u8; 32] {
        let mut out = [0u8; 32];

        for (out, (left, right)) in out.iter_mut().zip(self.0.iter().zip(other.0.iter())) {
            *out = left ^ right;
        }

        out
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }

    pub fn from_hex(hex: &str) -> Result<Self, String> {
        if hex.len() != 64 {
            return Err(format!("invalid NodeId hex length: {}", hex.len()));
        }
        // Byte slicing below needs ASCII, and from_str_radix alone would
        // accept a leading sign.
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid NodeId hex digit".into());
        }

        let mut out = [0u8; 32];

        for i in 0..32 {
            let chunk = &hex[i * 2..i * 2 + 2];
            out[i] = u8::from_str_radix(chunk, 16)
                .map_err(|e| format!("invalid NodeId hex byte '{}': {}", chunk, e))?;
        }

        Ok(Self(out))
    }
}

pub fn bucket_index(a: &NodeId, b: &NodeId) -> usize {
    let dist = a.xor_distance(b);

    for (i, byte) in dist.iter().enumerate() {
        if *byte != 0 {
            return i * 8 + byte.leading_zeros() as usize;
        }
    }

    255
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_is_stable_for_same_input() {
        let a = NodeId::from_name_address("wolf-a", "127.0.0.1:9560");
        let b = NodeId::from_name_address("wolf-a", "127.0.0.1:9560");

        assert_eq!(a, b);
    }

    #[test]
    fn node_id_changes_for_different_input() {
        let a = NodeId::from_name_address("wolf-a", "127.0.0.1:9560");
        let b = NodeId::from_name_address("wolf-b", "127.0.0.1:9561");

        assert_ne!(a, b);
    }

    #[test]
    fn public_key_id_is_domain_separated_and_key_bound() {
        let a = NodeId::from_public_key(&[1u8; 32]);

        assert_eq!(a, NodeId::from_public_key(&[1u8; 32]));
        assert_ne!(a, NodeId::from_public_key(&[2u8; 32]));
        assert_ne!(a.0, <[u8; 32]>::from(Sha256::digest([1u8; 32])));
    }

    #[test]
    fn hex_round_trips_and_rejects_non_hex_without_panicking() {
        let id = NodeId::from_public_key(&[7u8; 32]);

        assert_eq!(NodeId::from_hex(&id.to_hex()), Ok(id));
        assert!(NodeId::from_hex(&format!("+{}", "0".repeat(63))).is_err());
        assert!(NodeId::from_hex(&format!("é{}", "0".repeat(62))).is_err());
        assert!(NodeId::from_hex(&"0".repeat(63)).is_err());
    }

    #[test]
    fn bucket_index_self_is_last_bucket() {
        let a = NodeId::from_name_address("wolf-a", "127.0.0.1:9560");

        assert_eq!(bucket_index(&a, &a), 255);
    }
}
