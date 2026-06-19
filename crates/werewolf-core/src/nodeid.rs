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

    pub fn xor_distance(&self, other: &NodeId) -> [u8; 32] {
        let mut out = [0u8; 32];

        for i in 0..32 {
            out[i] = self.0[i] ^ other.0[i];
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
    fn bucket_index_self_is_last_bucket() {
        let a = NodeId::from_name_address("wolf-a", "127.0.0.1:9560");

        assert_eq!(bucket_index(&a, &a), 255);
    }
}
