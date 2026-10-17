# WerewolfProxy — Architecture, Security Model and Final Objective

## 1. Purpose of this document

This document describes the **overall architecture, security philosophy, operational model, and final objective of WerewolfProxy**.

It is not a changelog, implementation diary, or milestone-specific handoff.

Its purpose is to allow another engineer or AI system to understand:

- what WerewolfProxy is;
- what problem it is intended to solve;
- how its trust model works;
- how peers communicate;
- how services are exposed and consumed;
- how discovery and routing are intended to evolve;
- what Pack, Pelt, Fang, Silver, Den, Authority, Service Registry, and KAD mean;
- why root must eventually become unobtainable;
- how threshold physical-key governance is intended to work;
- what happens when a Werewolf network becomes unrecoverable;
- why anti-forensics and the final Tombstone are part of the design;
- which architectural boundaries must never be violated.

The central design goal is:

> **A Werewolf network must be able to become autonomous, self-sufficient, cryptographically governed, and operational without any permanent superuser capable of overriding its security model.**

---

# 2. What WerewolfProxy is

WerewolfProxy is intended to become a **secure peer-to-peer overlay network and service transport fabric**.

At its simplest, two Werewolf nodes should be able to establish a trusted connection and carry arbitrary TCP-based application traffic between them.

Examples include:

- SSH;
- HTTP;
- HTTPS;
- generic TCP services;
- internal administration interfaces;
- private APIs;
- remote-support services;
- application-specific protocols.

The application using Werewolf should not need to understand the cryptographic or routing machinery beneath it.

Conceptually:

```text
Local Application
       │
       ▼
Local Werewolf Node
       │
       ▼
Encrypted / authenticated Werewolf overlay
       │
       ▼
Remote Werewolf Node
       │
       ▼
Remote target service
```

For example:

```text
ssh localhost:2222
       │
       ▼
Werewolf Fang
       │
       ▼
Werewolf encrypted transport
       │
       ▼
wolf-a
       │
       ▼
127.0.0.1:22
```

The same mechanism should work for HTTP:

```text
curl http://127.0.0.1:18087
       │
       ▼
Werewolf
       │
       ▼
wolf-a
       │
       ▼
127.0.0.1:8080
```

or any other TCP service.

---

# 3. Final vision

The final system is not intended to be merely an encrypted port-forwarder.

The intended end state is a **self-organising, distributed, zero-trust private network** with:

- cryptographic node identities;
- authenticated peer membership;
- decentralized peer discovery;
- decentralized service discovery;
- encrypted peer-to-peer transport;
- policy-controlled service access;
- multi-hop routing;
- distributed routing knowledge;
- no permanent central controller;
- no dependency on a cloud vendor;
- no TPM dependency;
- no permanent root recovery path;
- threshold physical-key governance for exceptional ceremonies;
- explicit lifecycle states;
- fail-closed behaviour;
- secure destruction if the network becomes unrecoverable;
- a final Tombstone that records the network's death without retaining the network itself.

The long-term conceptual architecture is:

```text
                   ┌──────────────────────────┐
                   │     User / Application   │
                   └────────────┬─────────────┘
                                │
                                ▼
                   ┌──────────────────────────┐
                   │      Service Layer       │
                   │ names, discovery, intent │
                   └────────────┬─────────────┘
                                │
                                ▼
                   ┌──────────────────────────┐
                   │    Routing / KAD Layer   │
                   │ peer + service discovery │
                   └────────────┬─────────────┘
                                │
                                ▼
                   ┌──────────────────────────┐
                   │      Fang Transport      │
                   │ authenticated tunnels    │
                   └────────────┬─────────────┘
                                │
                                ▼
                   ┌──────────────────────────┐
                   │ Security / Authority     │
                   │ Pack / Policy / Silver   │
                   └────────────┬─────────────┘
                                │
                                ▼
                   ┌──────────────────────────┐
                   │ Cryptographic Identity   │
                   │          Pelt            │
                   └──────────────────────────┘
```

---

# 4. Fundamental security philosophy

Werewolf is designed around several non-negotiable principles.

## 4.1 Zero trust

No peer is trusted merely because:

- it is on the same LAN;
- it knows an IP address;
- it can connect to a port;
- it has previously participated in the network;
- it possesses a hostname;
- it claims a known name.

Trust must derive from cryptographic identity and explicit authorization.

---

## 4.2 Fail closed

When Werewolf cannot establish that an operation is valid, it should reject the operation.

Examples:

```text
missing identity          → reject
invalid peer key          → reject
unknown Pack member       → reject
invalid target policy     → reject
malformed state           → reject startup
ambiguous persistence     → enter degraded mode
Silver active             → reject new Fangs
```

The system must not silently convert damaged or ambiguous security state into permissive defaults.

---

## 4.3 No hidden bypass

No convenience layer may become an authorization bypass.

For example:

```text
Service Registry
```

may answer:

```text
"where is service web?"
```

but must never answer:

```text
"therefore you are authorized to use it"
```

Authorization remains with the existing security path.

---

## 4.4 No permanent omnipotent administrator

The final architecture rejects the conventional model:

```text
root
  ↓
can override everything
```

The desired mature Werewolf network instead becomes:

```text
network active
    │
    ▼
no single actor can obtain universal control
```

The long-term objective is:

> **Root becomes unobtainium.**

Once the network is activated and the security ceremony completed, there should be no ordinary operational mechanism that grants a person unrestricted root-like control over the whole network.

---

# 5. Core terminology

Werewolf deliberately uses a thematic vocabulary.

The terminology is architectural, not merely cosmetic.

---

## 5.1 Pelt

A **Pelt** is the cryptographic identity of a Werewolf node.

Conceptually it contains the material necessary to establish:

- node identity;
- public-key authentication;
- cryptographic fingerprint;
- TLS identity derivation where applicable.

A Pelt is not merely a hostname.

```text
wolf-a
```

is a human-readable name.

The Pelt proves which cryptographic entity that node actually is.

Conceptually:

```text
Node
 │
 └── Pelt
      ├── private identity material
      ├── public key
      └── fingerprint
```

The current system derives runtime transport identity from the Pelt.

---

## 5.2 Pack

The **Pack** is the set of peers recognized by a node as legitimate network participants.

A Pack entry contains information such as:

```text
name
fingerprint
public key
address
trust level
```

Membership in the Pack establishes peer identity/trust context.

It does not automatically authorize every service or target.

Conceptually:

```text
Pelt answers:
"Who are you?"

Pack answers:
"Are you one of the peers I recognize?"
```

---

## 5.3 Fang

A **Fang** is an active service/tunnel path through Werewolf.

A Fang connects:

```text
local listening endpoint
        │
        ▼
Werewolf transport
        │
        ▼
remote peer
        │
        ▼
remote target
```

Example:

```text
127.0.0.1:18087
        │
        ▼
       Fang
        │
        ▼
      wolf-a
        │
        ▼
127.0.0.1:8080
```

Fangs must always pass through the normal authorization machinery.

---

## 5.4 Fang Profile

A Fang Profile is a persistent description of a Fang configuration.

Conceptually:

```text
profile name
peer
local endpoint
remote endpoint
transport
```

Profiles are security-relevant because they can represent persistent activation intent.

For this reason they currently belong to the protected security-state group.

---

## 5.5 Service

A **Service** is a higher-level semantic name representing something offered by a peer.

Example:

```text
web
  peer   = wolf-a
  target = 127.0.0.1:8080
  kind   = http
```

The Service Registry answers:

```text
"where should 'web' resolve?"
```

It does NOT answer:

```text
"am I allowed to access web?"
```

That distinction is fundamental.

---

## 5.6 Den

The **Den** is the local private state directory used by the Werewolf daemon.

It stores local state using hardened filesystem semantics.

Examples include:

```text
pelt.json
services.json
security_state_current.json
security_state_{a,b}_manifest.json
security_state_{a,b}_pack.json
security_state_{a,b}_silver.json
security_state_{a,b}_target-policy.json
security_state_{a,b}_fangs.json
security_state_{a,b}_active-fangs.json
```

Since Stage15B the security-relevant documents are stored as two complete
generation slots (`a`, `b`) with per-generation manifests and a selector
(`security_state_current.json`); see section 8.1. Older Dens used flat files
such as `pack.json` and `fangs.json` and are converted by
`state manifest-migrate`.

The Den is not merely a configuration folder.

It is the local durable state boundary.

It uses mechanisms such as:

- private directory permissions;
- safe path handling;
- atomic replacement;
- collision detection;
- lifetime locking;
- protection against unsafe symlink/hard-link behaviour.

---

## 5.7 Silver

**Silver** is the emergency security state.

Conceptually:

```text
Silver active
     │
     ├── new Fangs rejected
     ├── existing authority fenced/cancelled
     └── network-facing activity constrained
```

Silver is not just a GUI "disable" button.

It participates in the authority lifecycle and security-state persistence.

Silver must fail safe.

---

## 5.8 Authority

The Authority subsystem controls active authorization state for network sessions.

It handles concepts including:

- peer authorization epochs;
- session leases;
- revocation;
- cancellation;
- transport-specific authority;
- Silver fencing.

A Pack record alone is not enough.

Runtime authority also matters.

---

## 5.9 Tombstone

The **Tombstone** is the final artifact left when a Werewolf network irreversibly dies.

The design principle is:

> Death leaves a gravestone, nothing more.

If the network reaches a formally unrecoverable state, the final destruction mechanism should eliminate recoverable network secrets and operational evidence while preserving only the deliberately designed Tombstone.

The Tombstone should not itself contain enough information to reconstruct the network.

---

# 6. Local control architecture

The daemon is the authoritative runtime.

The CLI is intentionally thin.

Current architecture:

```text
werewolfctl
     │
     │ ControlRequest
     ▼
Unix control socket
     │
     ▼
werewolfd
     │
     ├── state
     ├── policy
     ├── persistence
     ├── authority
     └── transports
```

The CLI should not independently implement security logic.

Its job is primarily to:

- parse human input;
- construct control requests;
- display daemon responses.

The daemon remains authoritative.

---

# 7. Mutation architecture

Persistent administrative mutations go through a single mutation coordinator.

The intended invariant is:

```text
candidate state
      │
      ▼
validate
      │
      ▼
durably persist
      │
      ▼
publish to live memory
```

Never:

```text
publish
  ↓
attempt persistence
  ↓
maybe fail
```

The correct order prevents runtime state from claiming a change that does not survive restart.

Conceptually:

```text
candidate
    │
    ▼
disk commit
   / \
fail  success
 │      │
 ▼      ▼
old    publish
live   candidate
```

---

# 8. Persistence model

Werewolf currently distinguishes between security-protected state and other durable Den state.

## 8.1 Protected security state

The security-state manifest currently covers:

```text
Silver
Pack
Target Policy
Fang Profiles
Active Fangs
```

These documents form a security-relevant policy group.

The manifest mechanism uses complete generations and an activation pointer to detect partial or mixed restoration.

Conceptually:

```text
generation N
 ├── Pack
 ├── Silver
 ├── Target Policy
 ├── Fang Profiles
 └── Active Fangs
```

A generation must be internally coherent.

---

## 8.2 Independent Den state

Other durable documents may live in the Den without belonging to the security-state manifest.

Examples:

```text
pelt.json
services.json
```

This does not mean they are written casually.

They still use the hardened Den storage mechanisms.

The distinction is:

```text
ProtectedState
    = security-authoritative policy group

Independent Den state
    = durable local state not itself granting authority
```

---

# 9. Transport architecture

Werewolf already has working transport paths.

The current architecture supports secure tunnels over:

- QUIC;
- encrypted TCP.

Plain TCP still exists in the code, but only as an explicit local choice
(`fang create ... --transport tcp-plain`, or the LEGACY selector). It has no
Werewolf encryption or peer authentication and forwards directly to the
target. The default secure selector never falls back to it: QUIC, then
encrypted TCP, then failure. See [V1 wire contract](V1_WIRE_CONTRACT.md).

The same high-level Fang abstraction should survive regardless of transport.

Conceptually:

```text
                Fang
                 │
         ┌───────┴────────┐
         ▼                ▼
       QUIC          encrypted TCP
```

Transport choice must not change the security semantics.

Both must still enforce:

```text
Pelt identity
Pack
Authority
Target Policy
Silver
resource limits
```

---

# 10. Target authorization

A remote peer being trusted does not imply unlimited access to the receiver.

Werewolf separates:

```text
peer trust
```

from:

```text
target authorization
```

This is essential.

Example:

```text
wolf-a is a Pack member
```

does not automatically mean:

```text
wolf-a may connect to every localhost service
```

Target Policy controls which targets are authorized.

Conceptually:

```text
Authenticated peer
       │
       ▼
Target Policy
       │
   ┌───┴────┐
   │        │
 allow     deny
```

---

# 11. Admission and resource control

Authentication alone must not grant unlimited computational or network resources.

Werewolf therefore also enforces bounded admission.

Examples include limits on:

- simultaneous handshakes;
- per-peer work;
- target connection work;
- QUIC contexts;
- reservations;
- control clients;
- administrative mutations.

This protects the system against trusted-but-abusive or compromised peers.

The rule is:

> A Pack member is authenticated, not entitled to unlimited work.

---

# 12. Service Registry architecture

The Service Registry introduces human-meaningful service names.

Example:

```text
web
ssh-admin
camera-01
git
monitoring
```

Each local service definition currently has:

```text
name
peer
target
kind
```

Example:

```json
{
  "name": "web",
  "peer": "wolf-a",
  "target": "127.0.0.1:8080",
  "kind": "http"
}
```

The intended command eventually becomes:

```bash
werewolfctl service connect web --local-port 18087
```

Resolution should occur in the daemon:

```text
service.connect web
       │
       ▼
lookup Service Registry
       │
       ▼
wolf-a + 127.0.0.1:8080
       │
       ▼
open_fang_from_parts()
```

This is preferable to resolving in the CLI because it keeps the daemon authoritative and avoids a lookup/open TOCTOU split.

---

# 13. Service Registry is not authority

This rule must never be broken.

A Service Registry entry:

```text
web → wolf-a:127.0.0.1:8080
```

means only:

```text
"web resolves to this peer and target"
```

The resulting connection still passes through:

```text
Pack validation
      ↓
Pelt / transport authentication
      ↓
Target Policy
      ↓
Authority
      ↓
Silver state
      ↓
Fang transport
```

Therefore a service may legitimately be configured before its peer joins the Pack.

The configuration can exist.

The connection cannot succeed until the normal authorization path permits it.

---

# 14. Service types

Current conceptual service kinds are:

```text
tcp
http
https
ssh
```

Initially these are mostly semantic classifications.

They must not create separate authorization paths.

For example:

```text
kind = ssh
```

does not mean the Service subsystem directly opens an SSH connection.

It still creates a Fang to the registered target.

---

# 15. Peer discovery

Werewolf is intended to evolve away from manually configured peer addresses.

The core already contains Kademlia-inspired components:

```text
NodeId
Peer
KBucket
RoutingTable
PeerStore
Lookup
FindNode
FindNodeResult
```

The long-term network should therefore become capable of discovering peers through a distributed routing structure.

Conceptually:

```text
Local node
   │
   ▼
known peers
   │
   ▼
FindNode
   │
   ▼
closer peers
   │
   ▼
target peer discovered
```

---

# 16. Node IDs and distance

Nodes have deterministic IDs.

Routing uses XOR-style distance.

Conceptually:

```text
distance(A, B) = A XOR B
```

This allows peers to organize routing knowledge into Kademlia-style buckets.

The distributed network therefore does not need one central peer directory.

---

# 17. K-buckets

Each node maintains only a bounded subset of knowledge about the network.

Conceptually:

```text
local node
  ├── bucket 0
  ├── bucket 1
  ├── bucket 2
  ├── ...
  └── bucket N
```

Buckets represent peers at increasing XOR-distance ranges.

This supports scalable decentralized lookup.

---

# 18. FindNode RPC

The core already defines message types for:

```text
Ping
Pong
FindNode
FindNodeResult
```

The future transport path should run these RPCs over authenticated Werewolf transport, ideally QUIC where appropriate.

Conceptually:

```text
wolf-a
   │ FindNode(target)
   ▼
wolf-b
   │
   └── returns nearest known peers
```

Repeated lookup converges toward the target.

---

# 19. Service discovery over KAD

Peer discovery is only one part of the final design.

Eventually the distributed overlay should also resolve services.

Conceptually:

```text
"who provides service X?"
          │
          ▼
distributed discovery
          │
          ▼
candidate providers
          │
          ▼
select peer
          │
          ▼
normal Werewolf authorization
```

The discovery layer can answer:

```text
who?
where?
what service?
```

It must never answer:

```text
authorized?
```

That remains local policy and cryptographic authority.

---

# 20. Local registry versus distributed discovery

The current local Service Registry is intended to become the first layer of a broader service system.

Evolution:

```text
Stage 1
local static service registry

Stage 2
local registry + discovered peers

Stage 3
distributed service advertisements

Stage 4
distributed service lookup

Stage 5
routing to selected provider

Stage 6
multi-hop service transport
```

The local model therefore should not be designed in a way that prevents future distributed resolution.

---

# 21. Multi-hop routing

The mature Werewolf overlay should not require every peer to have direct reachability to every other peer.

Future architecture:

```text
wolf-a
   │
   ▼
wolf-b
   │
   ▼
wolf-c
   │
   ▼
wolf-d
```

A service consumer may reach a provider through intermediate Werewolf nodes.

This will require careful separation between:

- route discovery;
- route selection;
- transport forwarding;
- end-to-end identity;
- authorization;
- metadata exposure.

Intermediate nodes should learn no more than operationally necessary.

---

# 22. End-to-end versus hop-by-hop trust

Future multi-hop operation must not accidentally reduce the network to a chain of fully trusted proxies.

The intended direction is:

```text
routing intermediaries
      ≠
ultimate application authority
```

Where feasible, service identity and authorization should remain end-to-end even when packet forwarding is multi-hop.

This will require future protocol work.

---

# 23. No central cloud dependency

Werewolf must remain functional without depending on:

- vendor cloud accounts;
- proprietary discovery servers;
- centralized authentication services;
- third-party control planes.

Optional external infrastructure may exist, but the network architecture must not require a specific vendor to remain alive.

The network should become self-sufficient.

---

# 24. No TPM dependency

The security architecture must not require TPM-backed custody.

TPMs may be useful in some environments, but Werewolf's fundamental security model must not depend on them.

The project explicitly prefers designs that can operate using external cryptographic hardware and distributed ceremonies instead.

---

# 25. Physical key governance

For exceptional network governance, the intended design uses physical cryptographic keys rather than a single master credential.

The current conceptual requirement is:

```text
10 physical keys total
7 required
```

That is:

```text
7-of-10 threshold
```

The objective is to avoid:

```text
one stolen key
        ↓
network compromise
```

or:

```text
one administrator
        ↓
total authority
```

Instead:

```text
7 independent participants / key holders
        ↓
exceptional ceremony
```

This mechanism is intended for rare high-authority lifecycle operations, not normal day-to-day connections.

---

# 26. Why threshold governance matters

Physical keys can be:

- stolen;
- seized;
- destroyed;
- lost;
- compromised.

A single physical key therefore cannot represent absolute control.

A 7-of-10 design provides tolerance against some lost or compromised keys while still requiring a strong quorum.

Conceptually:

```text
0..6 keys
  ↓
insufficient authority

7..10 valid keys
  ↓
ceremony may proceed
```

The exact cryptographic threshold mechanism remains an implementation area, but the governance objective is already decided.

---

# 27. Root must become unobtainium

This is one of the project's defining goals.

Traditional infrastructure assumes:

```text
someone can always become root
```

Werewolf intends to invert that assumption.

After activation:

```text
normal administration
      │
      ▼
bounded APIs / policies / ceremonies
```

not:

```text
sudo -i
```

The final network should not depend on a permanently recoverable superuser.

Even maintainers should be constrained by the architecture.

---

# 28. Why eliminate root recovery

A permanent root recovery mechanism is also a permanent compromise mechanism.

If a hidden universal override exists, then:

- theft can obtain it;
- coercion can obtain it;
- malware can target it;
- an insider can abuse it;
- operational mistakes can invoke it.

Therefore the mature architecture aims for:

> No permanent universal override after activation.

Exceptional changes must instead require explicit distributed ceremonies.

---

# 29. Activation ceremony

A mature Werewolf deployment is expected to have a meaningful transition between:

```text
installation / bootstrap
```

and:

```text
autonomous active network
```

During bootstrap, some privileged configuration may still be necessary.

The activation ceremony should ultimately:

- establish initial identities;
- establish Pack membership;
- establish policy;
- establish threshold governance;
- validate participating nodes;
- remove bootstrap superuser capability;
- transition to the autonomous trust model.

After this point, root should no longer be a normal recovery mechanism.

---

# 30. Autonomous operation

Once active, the network should be self-sufficient.

Normal operations should include:

- peer discovery;
- service discovery;
- routing;
- authorization;
- session creation;
- policy-controlled updates;
- peer revocation;
- Silver activation;
- controlled lifecycle changes.

None should require a central root login.

---

# 31. Network recoverability

The network must explicitly distinguish:

```text
temporarily unavailable
```

from:

```text
cryptographically unrecoverable
```

Temporary failure might include:

- nodes offline;
- route unavailable;
- minority of governance keys unavailable;
- network partition.

Unrecoverable means the required trust/governance conditions can no longer ever be satisfied.

Example conceptual condition:

```text
remaining valid governance capability < threshold
```

combined with no permitted recovery path.

---

# 32. Death of a Werewolf network

If the network becomes formally unrecoverable, it should not remain indefinitely in a half-dead state containing sensitive material.

The chosen design principle is:

> If the network cannot recover legitimately, it must die deliberately.

The death process should be an explicit part of the architecture, not an improvised administrative action.

---

# 33. Anti-forensics on network death

The project explicitly intends the final death sequence to include anti-forensics.

When an unrecoverable network executes its terminal lifecycle:

```text
network death
     │
     ▼
destroy operational secrets
     │
     ▼
remove recoverable network state
     │
     ▼
eliminate evidence sufficient to reconstruct network operation
     │
     ▼
leave Tombstone only
```

Users/operators must know and accept this behaviour beforehand.

This cannot be a surprise emergency feature.

It is part of the network's lifecycle contract.

---

# 34. The Tombstone principle

The final state is:

```text
death leaves a gravestone, nothing more
```

The Tombstone may record limited information such as:

- that a Werewolf network existed;
- its lifecycle identifier;
- the fact that terminal destruction occurred;
- possibly a cryptographic attestation of the terminal event.

It must not preserve:

- private keys;
- recoverable Pack state;
- service topology;
- useful routing information;
- session secrets;
- credentials;
- reconstructable operational state.

The precise Tombstone format remains future protocol work.

---

# 35. Silver versus Death

Silver and network death are different.

Silver is an emergency operational lock:

```text
Silver
  ↓
network/security activity fenced
  ↓
state remains recoverable
```

Death is terminal:

```text
Death
  ↓
security material destroyed
  ↓
network cannot resume
  ↓
Tombstone remains
```

This distinction must remain explicit in the lifecycle state machine.

---

# 36. Lifecycle model

A future lifecycle may conceptually resemble:

```text
BOOTSTRAP
    │
    ▼
CEREMONY
    │
    ▼
ACTIVE
    │
    ├────────► SILVER
    │             │
    │             └────► ACTIVE
    │
    ├────────► DEGRADED
    │
    └────────► UNRECOVERABLE
                    │
                    ▼
                  DEATH
                    │
                    ▼
                TOMBSTONE
```

The exact state machine is not final, but this illustrates the intended separation.

---

# 37. Storage degradation

The current implementation already recognizes an important intermediate condition:

```text
persistent commit outcome is indeterminate
```

This is treated differently from a clean failure.

If a mutation may have crossed the durable rename boundary but its final persistence status cannot be established, the daemon sets:

```text
storage_degraded = true
```

and disables further administrative mutations.

This reflects a core principle:

> Ambiguity in durable security state is itself a security event.

---

# 38. Control-plane safety

The local Unix control socket is itself bounded and validated.

The control protocol includes:

- strict JSON parsing;
- duplicate-key rejection;
- request-size limits;
- depth limits;
- command-specific allowed fields;
- client limits;
- request limits;
- lifetime bounds;
- mutation admission bounds.

Administrative interfaces are therefore treated as attack surfaces, not trusted plumbing.

---

# 39. Transport identity

Werewolf transport identity is tied to the Pelt.

TLS/transport configuration is intentionally constrained.

Current testing includes properties such as:

- TLS 1.3 only;
- expected Pelt certificate identity;
- rejection of wrong peer identities;
- no unintended semantic metadata in certificates;
- disabled resumption where required;
- disabled early data;
- controlled ALPN/SNI behaviour.

The exact transport mechanisms may evolve, but transport identity must continue to bind back to the node's cryptographic identity.

---

# 40. Replay resistance

The system includes explicit replay-resistance work.

Transport openings bind relevant context such as:

- peer identity;
- connection;
- stream/context;
- target/open semantics;
- cryptographic exporter/transcript state.

The architecture must continue preventing a valid request captured in one context from being replayed in another.

---

# 41. Revocation

Removing/revoking a Pack peer is not just a file edit.

It also interacts with runtime authority.

The system is designed so that revocation can:

- deny new authority;
- terminate relevant active Fangs;
- prevent stale authority tickets from becoming valid again;
- preserve runtime denial even around persistence failure where necessary.

This is why authorization state is not reducible to static JSON alone.

---

# 42. Local peer prototype and transition to native Rust

Earlier development used shell tooling for peer management and routing experiments.

Examples included:

```text
peer-db.sh
peer-gate.sh
peer-contract.sh
peer-monitor.sh
```

This shell layer served as a behavioural prototype.

The architectural direction is to move proven semantics into:

```text
werewolf-core
werewolfd
werewolfctl
```

and retire shell logic as authoritative implementation.

The shell scripts remain useful as integration gates during migration.

---

# 43. Multiwolf development environment

The project uses a local multi-node environment for integration testing.

Typical nodes:

```text
wolf-a
wolf-b
wolf-c
wolf-d
```

with separate:

- control sockets;
- TCP listener ports;
- QUIC listener ports;
- Den directories.

This environment is important because Werewolf functionality must be tested as an actual distributed system, not solely with unit tests.

---

# 44. Current service-transport maturity

Already demonstrated:

```text
HTTP through Werewolf      ✅
Generic TCP through Werewolf ✅
TCP half-close correctness ✅
QUIC transport             ✅
encrypted TCP transport    ✅
named local service registry CRUD ✅ (branch codex/stage23-service-registry)
```

SSH integration exists as a gate and depends on availability of a local SSH service during testing.

---

# 45. Current Service Registry development stage

At the current development point:

Implemented:

```text
ServiceDefinition
ServiceKind
services.json startup restoration
service.add
service.remove
service.list
service.show
```

The persistent registry has been designed as:

```text
non-authoritative but durable
```

Startup validation is fail-closed.

CRUD persistence semantics are certified by tests: mutations persist
`services.json` before publishing to live state, failed saves leave memory
unchanged, and `service.add` does not require the peer to be a Pack member.

The next immediate work is to implement daemon-native:

```text
service.connect <name>
```

---

# 46. Intended service.connect architecture

The correct future path is:

```text
werewolfctl service connect web --local-port 18087
                    │
                    ▼
ControlRequest service.connect
                    │
                    ▼
werewolfd
                    │
                    ▼
lookup "web"
                    │
                    ▼
peer = wolf-a
target = 127.0.0.1:8080
                    │
                    ▼
open_fang_from_parts()
                    │
                    ▼
existing security path
```

Not:

```text
CLI lookup
  ↓
CLI independently builds authorization logic
```

The daemon remains authoritative.

---

# 47. Future service advertisements

Once distributed service discovery is introduced, a node may advertise services into the overlay.

Conceptually:

```text
wolf-a advertises:
  web
  git
  ssh-admin
```

A peer may discover:

```text
service web
providers:
  wolf-a
  wolf-f
  wolf-k
```

Provider selection may eventually consider:

- XOR/routing proximity;
- latency;
- availability;
- reputation;
- policy;
- service metadata.

But discovery still does not grant authorization.

---

# 48. Reputation and availability

The experimental peer model already contains ideas such as:

```text
healthy
reputation
availability_pct
latency_ms
```

These may later inform route/provider selection.

They should not become substitutes for cryptographic identity.

A highly available unknown peer is still unknown.

---

# 49. Routing policy

Future route selection should distinguish:

```text
can route through peer?
```

from:

```text
can access final service?
```

Routing eligibility and application authorization are separate concepts.

This separation becomes especially important under multi-hop transport.

---

# 50. Privacy direction

As the system evolves, routing and service discovery should minimize unnecessary metadata disclosure.

Potential future concerns include:

- hiding full service topology from unrelated peers;
- minimizing route metadata;
- preventing intermediaries from learning application payloads;
- limiting discoverability of sensitive services;
- authenticating advertisements;
- preventing malicious service-name poisoning.

These are future protocol design areas.

---

# 51. No vendor lock-in

The final WerewolfProxy should remain:

- self-hostable;
- protocol-controlled by the project;
- independent of proprietary identity clouds;
- independent of proprietary overlay networks;
- portable across systems.

The project should prefer open, documented formats and clean-room implementations where external interoperability is needed.

---

# 52. Platform direction

The current implementation is strongly Linux-oriented, particularly around:

- Unix sockets;
- filesystem security;
- daemon model;
- control-plane semantics.

Linux CLI support is the first-class development target.

GUI and mobile clients are later layers.

They must use the same daemon/network security model rather than inventing separate security semantics.

---

# 53. Future GUI/mobile architecture

Eventually:

```text
GUI / Mobile
      │
      ▼
stable Werewolf control API
      │
      ▼
Werewolf daemon / node
```

The GUI should be presentation and orchestration.

It should not hold hidden universal credentials or become a privileged bypass.

---

# 54. Operational UX objective

Despite the strong internal security model, normal use should ultimately be simple.

For example:

```bash
werewolfctl service list
```

might show:

```text
web
git
camera-gate
ssh-admin
```

and:

```bash
werewolfctl service connect web --local-port 8080
```

should be sufficient for the operator.

Complexity belongs inside the system.

Security must not require the user to manually reproduce cryptographic procedures for every ordinary connection.

---

# 55. Separation of planes

The mature architecture can be understood as several planes.

## Identity plane

```text
Pelt
```

Answers:

```text
Who is this node?
```

## Membership plane

```text
Pack
```

Answers:

```text
Which peers are recognized?
```

## Authorization plane

```text
Target Policy
Authority
Silver
```

Answers:

```text
May this action occur now?
```

## Service plane

```text
Service Registry / discovery
```

Answers:

```text
What does this service name resolve to?
```

## Routing plane

```text
KAD / routing table / multi-hop
```

Answers:

```text
How can I reach that peer/provider?
```

## Transport plane

```text
Fang / QUIC / encrypted TCP
```

Answers:

```text
How are the bytes carried securely?
```

## Governance plane

```text
threshold hardware-key ceremonies
```

Answers:

```text
How are exceptional lifecycle changes authorized?
```

## Lifecycle plane

```text
Bootstrap / Active / Silver / Death / Tombstone
```

Answers:

```text
What state is the network itself in?
```

---

# 56. Architectural dependency direction

Higher-level convenience layers must depend on lower-level security layers, never the reverse.

Good:

```text
Service
  ↓
Routing
  ↓
Fang
  ↓
Authority
  ↓
Identity
```

Bad:

```text
Authority trusts Service Registry
```

or:

```text
Pack membership inferred from service advertisement
```

Security foundations must not depend on convenience metadata.

---

# 57. Final security invariant

The mature Werewolf network should maintain this invariant:

```text
No route,
service name,
cached discovery result,
GUI action,
or administrative convenience
can create authority that was not already granted
by the cryptographic and policy layers.
```

---

# 58. Final resilience invariant

The mature network should also maintain:

```text
loss of one machine
loss of one route
loss of one administrator
loss of one hardware key
```

must not automatically destroy the network.

But:

```text
loss of sufficient governance capability
```

must not silently create a backdoor recovery path either.

If legitimate recovery becomes impossible, terminal destruction is preferable to insecure resurrection.

---

# 59. Final anti-compromise philosophy

Traditional systems often optimize for:

```text
recover at all costs
```

Werewolf deliberately rejects that as an absolute rule.

The preferred priority is:

```text
security integrity
    >
illegitimate recoverability
```

Therefore:

```text
recover legitimately
or
die securely
```

but never:

```text
recover by hidden master bypass
```

---

# 60. Final objective in one flow

The ultimate Werewolf experience should look approximately like this:

```text
User asks for service "web"
          │
          ▼
local registry / distributed discovery
          │
          ▼
provider discovered
          │
          ▼
route discovered
          │
          ▼
peer cryptographically authenticated
          │
          ▼
Pack membership evaluated
          │
          ▼
target authorization evaluated
          │
          ▼
Authority + Silver evaluated
          │
          ▼
Fang established
          │
          ▼
application traffic flows
```

All of this should happen without:

- a central cloud controller;
- a permanent root operator;
- a hidden master password;
- a TPM requirement;
- a proprietary overlay provider.

---

# 61. Final governance objective

Exceptional actions should require a ceremony.

Conceptually:

```text
10 physical governance keys
          │
          ▼
7 valid participants required
          │
          ▼
ceremony
          │
          ▼
authorized exceptional state transition
```

No individual key holder should possess unilateral network sovereignty.

---

# 62. Final death objective

If the network becomes legitimately unrecoverable:

```text
detect unrecoverable condition
          │
          ▼
enter terminal lifecycle
          │
          ▼
destroy secrets
          │
          ▼
anti-forensic cleanup
          │
          ▼
destroy reconstructable network state
          │
          ▼
write Tombstone
          │
          ▼
network is dead
```

Nothing should remain that effectively recreates the old root problem.

---

# 63. The Werewolf design in one sentence

> **WerewolfProxy is intended to become a self-organising, cryptographically authenticated, policy-controlled P2P service network that can operate without centralized infrastructure or permanent superuser authority, and that prefers secure death over insecure recovery when its legitimate trust structure becomes irrecoverable.**

---

# 64. Architectural rules for future development

Any future implementation should preserve the following rules:

1. **Pelt is identity.**
2. **Pack is peer membership/trust context.**
3. **Service discovery is not authorization.**
4. **Routing is not authorization.**
5. **Fangs use the existing authorization path.**
6. **Target Policy limits remote service access.**
7. **Authority controls runtime authorization.**
8. **Silver is a real security fence, not UI state.**
9. **Persistent administrative state is validated before publication.**
10. **Ambiguous persistence fails safe.**
11. **No convenience subsystem may introduce an authorization bypass.**
12. **No TPM dependency may become fundamental.**
13. **No permanent root recovery mechanism should survive final activation.**
14. **Exceptional governance should be threshold-based.**
15. **Network death must be an explicit protocol state.**
16. **Terminal destruction leaves a Tombstone, not a recoverable network.**
17. **CLI/GUI are clients of the authoritative daemon/security model.**
18. **Distributed discovery must preserve local authorization sovereignty.**
19. **Multi-hop routing must not imply trust in intermediaries.**
20. **The network must be able to become self-sufficient.**

---

# 65. Current development direction

The near-term engineering path remains:

```text
Service Registry CRUD certification
        ↓
daemon-native service.connect
        ↓
named HTTP/TCP/SSH service gates
        ↓
FindNode RPC over authenticated transport
        ↓
distributed peer discovery
        ↓
distributed service discovery
        ↓
multi-hop routing
        ↓
mature governance/lifecycle machinery
        ↓
root-unobtainium activation model
        ↓
terminal death / Tombstone protocol
```

The implementation should evolve incrementally, but all stages should remain compatible with the final architecture described above.
