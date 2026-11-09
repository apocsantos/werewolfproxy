# Stage42 — Fang connection ceiling (survey)

**Status: SURVEY, documentation only.** No code until the owner decides
§9. The only change outside this document is a one-sentence correction
to `KNOWN_LIMITATIONS.md` (§3.2).

Source: Stage41 survey §7 and owner decision S41-D5.

## 1. Baseline

- `main` `610eb0e` (Stage41 closed, PR #118).
- tokio 1.52.3, quinn-proto 0.11.18. `v1.0.0-rc.2` frozen.

## 2. The five Fang forwarders

Every Fang kind runs the same shape of loop: accept a local TCP
connection, spawn a task for it, accept the next. **None of them limits
how many connections are open at once.**

| Fang kind | Accept loop | Each local connection makes | Accept error |
|---|---|---|---|
| QUIC (`quic_fang.rs:72`) | `loop { accept … spawn }` | a QUIC connection to the peer (initiator buffers up to 1.25 MB receive + 10 MB send, Stage41) | logged, `continue` at once |
| encrypted TCP (`tcp_encrypted.rs:301`) | same | an encrypted TCP connection to the peer | `?`: the Fang's loop **ends** |
| plain TCP (`tcp_plain.rs:28`) | same | a TCP connection to the target | `?`: ends |
| routed (`relay.rs:148`) | same | a QUIC or TCP connection to the relay | `?`: ends |
| failover (`failover.rs:173`) | same | up to one attempt per provider, in order | `?`: ends |

Each task lives until its session ends or the Fang is cancelled.

## 3. Who can open connections to a Fang

### 3.1 The bind address is not restricted (CONFIRMED from code)

A Fang's `local` address only has to parse as a `SocketAddr`
(`werewolf-core/src/state_validation.rs:131`; `fang.open` and
`fang.profile.add` pass it through). `0.0.0.0:…` or a LAN address is
accepted. Then **any host that can reach that address** can open
sessions through the Fang. The Fang does no authentication: it is the
operator's local entry point.

So "local clients" means whoever can reach the bind address. On
loopback that is local processes. On another address it can be the
whole network.

### 3.2 Correction to the Stage41 limitation

The Stage41 entry in `KNOWN_LIMITATIONS.md` says the Fang connection
count is unbounded "(local clients only)". That is true only for a
loopback bind. This PR corrects the sentence; no other text changes.

## 4. What a flood costs today

| Resource | Bound today | Evidence |
|---|---|---|
| Sessions accepted by **one remote peer** from this node | 64 (`PEER_SESSIONS`), 16 concurrent handshakes (`PEER_HANDSHAKES`), 1024 global on the receiver | CONFIRMED (`authority.rs:17–19`, `admission.rs:10–23`) |
| Local TCP sockets, tasks and connection attempts on **this** node | **none** | CONFIRMED |
| File descriptors | process limit (`LimitNOFILE=4096` in the shipped systemd unit) | CONFIRMED (`packaging/systemd/werewolfd.service:25`) |
| QUIC initiator memory | 1.25 MB receive per connection (Stage41) + up to 10 MB send, × number of connections | CONFIRMED per connection; total unbounded |
| Plain TCP | one target connection per local connection, no remote quota | CONFIRMED |

What follows is HYPOTHESIS, because nothing has measured it:

- **Self-denial of service.** Beyond 64 sessions to one peer, the
  receiver refuses the rest. The local node still pays a socket, a task
  and a handshake attempt for each one, and the operator's legitimate
  sessions to that peer compete for the same 64 slots.
- **File-descriptor exhaustion.** At the fd limit, `accept` fails with
  `EMFILE`. For four Fang kinds the `?` **ends the Fang's accept loop**:
  the Fang stops listening while still registered as open. The QUIC
  Fang instead retries `accept` with no delay, which may spin a CPU core
  until descriptors free up. Other daemon functions that need a
  descriptor (control socket, receiver, discovery) fail at the same
  time.
- **Memory.** QUIC Fang connections multiply the Stage41 per-connection
  bound.

## 5. Options

### 5.1 Ceiling

| Option | What | Notes |
|---|---|---|
| **A. Per-Fang ceiling** | one semaphore per active Fang | contains one Fang's flood; other Fangs unaffected |
| **B. Global ceiling** | one semaphore across all Fangs | protects the daemon's descriptors and memory as a whole |
| **A + B** | both | a connection needs a slot in both |
| none | — | today |

**Suggested values** (to be decided in D2): per Fang **64**, the receiver's
per-peer session quota, since more sessions to one peer are refused
anyway. Global **1024**, the receiver's global session quota, which stays
well under the shipped 4096-descriptor limit (each session uses about 2–3
descriptors).

### 5.2 What happens to a connection over the ceiling

| Option | Behaviour | Trade-off |
|---|---|---|
| **R. Accept and reset** | accept, then close with RST at once (the existing `arm_reset_on_close`) | the client fails fast and visibly; consistent with "never a session that looks complete"; costs one short-lived descriptor |
| **W. Stop accepting while full** | pause `accept` until a slot frees | the kernel backlog fills and clients hang until their own timeout; no reset semantics added |

Recommended: **R**. Clients get a fast, explicit failure, and the
existing reset rule covers it.

### 5.3 Accept errors (related, small)

The fd-exhaustion behaviour (§4) is a separate defect. Proposal: on an
`accept` error, every Fang kind logs it, waits briefly (e.g. 100 ms) and
keeps listening. No Fang should either exit or spin. This is in scope
only if D4 says so.

### 5.4 Bind address (recorded, not proposed)

Restricting `local` to loopback would change documented behaviour and
could break operators who share a Fang on a LAN on purpose. It is
recorded as a possible later decision. This stage only documents it
(§3.2).

## 6. Recommendation

**A + B with R, plus 5.3.** Per-Fang 64, global 1024. A connection over
either ceiling is accepted and reset at once. Accept errors back off and
never end the Fang. All of this sits in one shared helper used by the
five accept loops. The loops already share `arm_reset_on_close` and
`cancellation.spawn`, so the helper sits next to them in
`transport/mod.rs`.

Not changed: any receiver quota, any window or timeout, any wire
behaviour, the Stage41 bounds.

## 7. Proposed Stage42 contract (for owner review)

- **M1, reproduction first** (lab, disposable, no live wolves):
  - a daemon under `prlimit --nofile=256` with one Fang of each kind; a
    local client opens connections until `accept` fails. Record per
    kind: does the Fang keep listening, CPU while saturated, the
    descriptor count, and whether a new legitimate connection works once
    the flood stops;
  - one QUIC Fang flooded with 500 connections to a receiver that holds
    sessions open: count local tasks and sockets, and check whether a
    legitimate session gets through.
  - M1 stops the stage if none of the §4 hypotheses reproduces.
- **M2, fix:** §6 in one helper, with unit tests per behaviour:
  ceiling reached → reset; slot released → next accept works; accept
  error → back off and keep listening. Workspace tests, fmt, clippy,
  `scripts/ci-local.sh` full.
- **M3, regression:** the M1 scenarios again (bounded now), plus Stage32
  S4-P0/P2 (connection setup, 50 opens per run per transport, well under
  64) and the routed-Fang gate. Every historical verdict must hold.
- **M4:** record, `KNOWN_LIMITATIONS` update, closure.

## 8. Risks

- **A legitimate workload over 64 concurrent connections through one
  Fang** would now see resets. Today, connections to one peer beyond 64
  are refused by the receiver anyway, and plain-TCP Fangs have no such
  quota. D2 can set a different plain-TCP value if wanted.
- **Failover and routed Fangs:** the ceiling counts local connections,
  not provider attempts. One local connection holds one slot while the
  providers are tried in turn.

## 9. Owner decisions

| # | Decision | Options | Recommended |
|---|---|---|---|
| S42-D0 | Approve Stage42 as scoped (§7) | approve, amend, reject | approve |
| S42-D1 | Ceiling scope | A, B, A + B, none | A + B |
| S42-D2 | Values | per Fang 64 / global 1024; other | 64 / 1024 |
| S42-D3 | Over the ceiling | R (accept and reset), W (stop accepting) | R |
| S42-D4 | Include the accept-error back-off (5.3) | yes, separate stage, no | yes |
| S42-D5 | Loopback-only bind (5.4) | document only (this stage); later stage; never | document only |

**Not authorised by this survey:** implementing Stage42, any receiver
quota change, tokio promotion, rc.3, release certification.
