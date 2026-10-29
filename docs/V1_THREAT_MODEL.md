# WerewolfProxy v1 threat model

## Assets and boundaries

Assets include Pelt private identity, Pack trust, exact target policy, Silver
latch, active Fang intent, session cryptographic state, and availability of
the daemon. Network authentication uses TLS 1.3 exact-SPKI receiver binding and
Ed25519 sender proof. Target authorization and resource/authority limits are
separate from Pack membership.

Since rc.1 (Stage23–31) the assets also include the Service Registry and
published services, contact records, routes, the governance chain head and
roster (in the protected manifest), the statement archive, the auto-sync
switch, and the Tombstone of a dead or retired node. Governance private
keys belong to their holders and the offline tools; the daemon never
holds or needs one.

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
| Malicious relay (Stage26) | The inner session is TLS pinned to the destination and authorized by it end to end; the relay forwards only under its own Target Policy | It learns the requester's identity, the destination's listener address, timing and volume; it can deny, delay or drop |
| Malicious Pack peer answering discovery or sync (Stage24–31) | Contact records and statements are signed; a record is pinned to its key, a statement applies only with 7 valid governance signatures against the node's own head | Withholding or serving stale records (eclipse) or withholding statements; a node that never hears a revocation keeps trusting the revoked member |
| Compromised host in a governed network (Stage29) | After activation the roster bounds every other node's Pack, so one host cannot widen trust on any other | OS root on that host can rewrite its own Den and trust anyone itself |
| Holder of fewer than 7 governance keys (Stage28–30) | Nothing is accepted without 7 of 10 signatures; a conflicting valid statement fences the node (fork) | — |
| Holder of 7 or more governance keys | None, by definition: they are the network's authority | Can revoke every member or kill the network |
| Control-socket user on one node (Stage30) | Retirement needs the node's own fingerprint and affects only that node | Can retire that node, as it can already re-initialise it |

## Explicit nonclaims

WerewolfProxy does not protect secrets from a fully compromised privileged
host/root/kernel while running. If the application or target is compromised,
WerewolfProxy does not make it trustworthy. Stage15B consistency is not
freshness. Stage16/20 do not establish immunity to arbitrary denial of service.
There is no traffic-flow confidentiality, cover traffic, constant-rate
transmission or multi-hop anonymity: one relay hides the requester's
address from the destination, not the requester from the relay. Discovery
and governance sync add observable periodic traffic. Destruction on death
or retirement makes no claim about storage media, logs, swap, backups or
copies outside the Den, nor that any governance private key was destroyed.
No timer, clock or network observation ever triggers destruction. The
certified release platform is Linux x86_64 only.
