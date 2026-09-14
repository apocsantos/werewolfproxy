use super::*;
use zeroize::Zeroize;

#[test]
fn low_order_public_inputs_fail_closed_for_both_tcp_roles() {
    let mut one = [0u8; 32];
    one[0] = 1;
    for _ in 0..2 {
        let secret = StaticSecret::from(hs::random::<32>().unwrap());
        for peer_bytes in [[0u8; 32], one] {
            // The library accepts these encodings and calculates zero. Test
            // the actual DH result, then the common production KDF gate.
            let raw = secret.diffie_hellman(&X25519PublicKey::from(peer_bytes));
            assert_eq!(raw.as_bytes(), &[0u8; 32]);
            let error = match derive_shared_key(&secret, &STANDARD.encode(peer_bytes)) {
                Err(error) => error,
                Ok(_) => panic!("low-order point must be rejected"),
            };
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(error.to_string(), "handshake rejected");
        }
    }
}

#[test]
fn valid_dh_preserves_symmetric_blake3_kdf() {
    let client = StaticSecret::from(hs::random::<32>().unwrap());
    let server = StaticSecret::from(hs::random::<32>().unwrap());
    let client_public = X25519PublicKey::from(&client);
    let server_public = X25519PublicKey::from(&server);
    let client_key =
        derive_shared_key(&client, &STANDARD.encode(server_public.as_bytes())).unwrap();
    let server_key =
        derive_shared_key(&server, &STANDARD.encode(client_public.as_bytes())).unwrap();
    let original_digest = blake3::hash(client.diffie_hellman(&server_public).as_bytes());
    assert_eq!(&client_key[..], original_digest.as_bytes());
    assert_eq!(&server_key[..], original_digest.as_bytes());
    assert_ne!(&client_key[..], &[0u8; 32]);
}

#[test]
fn owned_session_key_can_be_explicitly_wiped_without_inspecting_freed_memory() {
    let mut key = Zeroizing::new(hs::random::<32>().unwrap());
    assert_ne!(&key[..], &[0u8; 32]);
    key.zeroize();
    assert_eq!(&key[..], &[0u8; 32]);
}
