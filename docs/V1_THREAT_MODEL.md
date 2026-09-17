# WerewolfProxy v1 threat model

## Assets and boundaries

Assets include Pelt private identity, Pack trust, exact target policy, Silver
latch, active Fang intent, session cryptographic state, and availability of
the daemon. Network authentication uses TLS 1.3 exact-SPKI receiver binding and
Ed25519 sender proof. Target authorization and resource/authority limits are
separate from Pack membership.

## Attacker classes

| Attacker | v1 protection | Not protected |
| --- | --- | --- |
| Passive network observer | TLS/session confidentiality and authenticated peers | Endpoints, transport, packet size, timing, volume and traffic relationships |
| Active network attacker | TLS and signed connection-bound authentication, target policy, replay checks | Network denial, live relay to intended receiver, endpoint compromise |
| Unauthenticated remote attacker | Finite admission/work limits and strict parsing | Distributed bandwidth/resource exhaustion or starvation under sustained saturation |
| Authenticated malicious Pack peer | Target grants, authority ceilings, replay and per-peer work limits | Abuse within that peer's allowed target/resource envelope |
| Local unprivileged user without Den write access | Stage11B owner/mode, file type, lock and control peer-credential checks | Compromised same-UID process and OS vulnerabilities |
| Root/admin or compromised kernel | No meaningful secret isolation claim while running | Can read/replace process and state, alter binaries or bypass OS protections |
| Offline Den restoration attacker | Partial protected-state inconsistency is detected | Full internally consistent historical Den replacement/freshness |
| Full VM/disk snapshot rollback attacker | No external monotonic anchor exists | Whole-machine rollback of Den, local markers, logs and counters |

## Explicit nonclaims

WerewolfProxy does not protect secrets from a fully compromised privileged
host/root/kernel while running. If the application or target is compromised,
WerewolfProxy does not make it trustworthy. Stage15B consistency is not
freshness. Stage16/20 do not establish immunity to arbitrary denial of service.
There is no traffic-flow confidentiality, cover traffic, constant-rate
transmission or multi-hop anonymity. The certified release platform is Linux
x86_64 only.
