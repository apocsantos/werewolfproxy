use crate::transport::FangTransport;

#[allow(dead_code)]
#[derive(Clone)]
pub(super) struct ExpectedPeerIdentity {
    pub(super) fingerprint: String,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum TransportPolicyError {
    BadTcpAddress,
    BadQuicAddress,
}

pub(super) fn select_peer_transport(
    requested: &str,
    peer_address: &str,
) -> Result<FangTransport, TransportPolicyError> {
    match requested {
        "tcp" | "tcp-plain" => {
            let address = peer_address.replace("quic://", "tcp://");
            match parse_fang_transport(&address) {
                Ok(transport @ FangTransport::Tcp(_)) => Ok(transport),
                _ => Err(TransportPolicyError::BadTcpAddress),
            }
        }
        _ => match parse_fang_transport(peer_address) {
            Ok(transport @ FangTransport::Quic(_)) => Ok(transport),
            _ => Err(TransportPolicyError::BadQuicAddress),
        },
    }
}

pub(super) fn is_plain_tcp(requested: &str) -> bool {
    requested == "tcp-plain"
}

pub(super) fn requested_transport(value: Option<&str>) -> String {
    value.unwrap_or("quic").trim().to_string()
}

fn parse_fang_transport(address: &str) -> Result<FangTransport, String> {
    if let Some(rest) = address.strip_prefix("tcp://") {
        return Ok(FangTransport::Tcp(rest.to_string()));
    }

    if let Some(rest) = address.strip_prefix("quic://") {
        return Ok(FangTransport::Quic(rest.to_string()));
    }

    Err("peer address must start with tcp:// or quic://".to_string())
}
