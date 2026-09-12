# Stage 11C Session Authority

Stage 11C adds runtime authority fencing for authenticated inbound TCP and
QUIC sessions without changing either wire protocol. Each authenticated OPEN
captures a global Silver epoch and a per-peer generation in an opaque,
connection-local ticket. A bounded registry retains establishing and active
leases; leases own cancellation and quota permits until cleanup completes.

Authority is checked after Pack membership, receiver binding, signature
verification, and transport admission. Target authorization and target
connection occur only after a lease is reserved. Publication and every raw
application write poll recheck the ticket under a short synchronous gate.
The gate never spans an await, filesystem operation, or daemon-state lock.
Bytes submitted before revocation may still be delivered by the kernel or
QUIC, but stale leases cannot submit new bytes or publish a session.

`pack.revoke` and `pack.remove` first install a runtime deny fence, cancel
matching establishing and active inbound sessions, and then persist the Pack
mutation. A failed write leaves the peer in
`RUNTIME_DENIED_PENDING_DURABILITY`; authority is never silently restored.
After durable removal the peer is `REVOKED`, and re-adding it receives a new
generation. Restart before durable removal may reload the previous Pack;
this remains the documented durability boundary.

Silver is persisted as `<Den>/silver.json` with the closed v1 schema
`{"version":1,"mode":"locked"}` or `{"version":1,"mode":"open"}`.
Missing state is locked. ON advances the global epoch and cancels all inbound
leases plus daemon-managed Fang work before durable publication. OFF requires
healthy storage and a durable open latch, then advances the epoch and permits
only fresh sessions; killed sessions never resume. Control access remains
available while locked.

Initial limits are 1024 combined establishing/active inbound sessions, 64 per
authenticated peer, and 64 streams per QUIC connection, with a five-second
cleanup watchdog. Active entries are never evicted. QUIC replay reservations
remain connection-owned and are independent of session cleanup.

Stage 11C intentionally preserves TCP/QUIC v3 transcripts, cryptography,
framing, replay and exporter semantics, Stage 9 target authorization,
Stage 11A selector behavior, and Stage 11B local security and persistence
guarantees. Already-submitted network bytes cannot be recalled; maximum
session lifetime, bandwidth quotas, wire opacity, and Windows support remain
deferred.
