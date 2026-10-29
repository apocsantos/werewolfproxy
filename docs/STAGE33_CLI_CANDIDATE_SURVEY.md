# Stage33 CLI architecture-complete candidate — survey

**Status: draft for the owner's decisions.** Docs only. No code, tag,
archive or version change until the decisions below are reviewed.

This is the last item on the owner's order (2026-10-26): Stage30
lifecycle → Stage31 automatic sync → Stage32 robustness → **the CLI
architecture-complete candidate**. The owner's condition, that robustness
certification come before calling the CLI architecture complete and before
final release claims, is met: Stage32 completed 2026-10-29 (network matrix,
T1 and T2; see [the spec](STAGE32_ROBUSTNESS_SPEC.md) and
[the time record](STAGE32_TIME_CERTIFICATION.md)).

The survey asks:

1. what "architecture complete" means, so the claim can be checked;
2. what form the candidate takes and what it is called;
3. what is frozen in the CLI contract;
4. what happens to the hidden commands;
5. which documents must be brought up to date first;
6. which gates the candidate must pass;
7. how upgrade from 1.0.0-rc.1 is checked;
8. what is explicitly out of scope.

## Where things stand

**The frozen release is 1.0.0-rc.1** (Stage21), on the Stage9–20 feature
set. Since then, on `main` and not in that archive:

| Plane (ARCHITECTURE §55, plus discovery) | Stages | State |
|---|---|---|
| Identity | Pelt (rc.1) | certified in rc.1 |
| Membership | Pack (rc.1); roster-bounded Pack (Stage29) | lab gates, WAN checked |
| Authorization | Target Policy, Authority, Silver (rc.1) | certified in rc.1 |
| Service | named services (23), service discovery (25), providers and failover (27) | lab gates, WAN checked |
| Routing | one operator-set relay (26) | lab gate, WAN checked |
| Transport | QUIC, encrypted TCP (rc.1); HC02, LC-01, F1, F4, F5 fixes | Stage22R WAN, Stage32 sandbox |
| Discovery | Pack-only `ping`/`find_node`, contact records (24) | lab gate, WAN checked |
| Governance | 7-of-10 statements (28), admission and activation (29), sync (29, 31) | lab gates, WAN/real-host checked |
| Lifecycle | death, retirement, destruction, Tombstone (30) | lab gate, real-host check |

Every plane has an implemented, operator-facing CLI path, and each stage
has a lab gate and a record. What has not happened since rc.1 is a release-level pass
over the whole: one audit, one consolidated contract, consistent
documents and one archive.

**What the audit for this survey found:**

- **CLI contract.** [`V1_CLI_CONTRACT.md`](V1_CLI_CONTRACT.md) is the rc.1
  contract plus one appended section per stage (Stage23 to Stage31). It is
  complete but layered: the rc.1 preamble still says "This contract
  freezes the visible Stage18 administrative commands", and the exit
  classes are spread over the sections (exit 67, `TOMBSTONE`, is the
  daemon's, Stage30).
- **Wire contract.** [`V1_WIRE_CONTRACT.md`](V1_WIRE_CONTRACT.md) has the
  Stage24 and Stage25 RPC extensions but no section for the Stage26 routed
  OPEN (relay) or the Stage29 governance RPC; those exist only in their
  own specs ([Stage29 RPC](STAGE29_GOVERNANCE_RPC_WIRE_SPEC.md)) and the
  Stage26 design.
- **Operator documents** last changed with rc.1 and mention none of
  Stage23–32: `README.md`, `QUICKSTART.md`, `docs/OPERATIONS.md`,
  `docs/DAILY_COMMANDS.md`, `SECURITY.md`, `TRANSPORT_ARCHITECTURE.md`
  (no relay), `docs/V1_THREAT_MODEL.md`, `docs/V1_SECURITY_PROPERTIES.md`.
- **ARCHITECTURE.md** is current in its planes and objectives but has
  stale progress sections: §34 ("The precise Tombstone format remains
  future protocol work", specified in Stage30), §44–46 (Service Registry
  "on branch codex/stage23-service-registry"), §65 (the development path
  from Stage23 on, all now done). Nothing in it describes Stage31's
  automatic sync beyond one line in §30.
- **KNOWN_LIMITATIONS.md** is the rc.1 list plus an "On `main` after
  1.0.0-rc.1" section, now current to Stage32.
- **Versions.** The binaries report `WEREWOLF_RELEASE_VERSION` when it is
  set at build time; the three Cargo packages are still `0.1.0` (a known
  rc.1 distinction).
- **Hidden commands** (clap `hide = true`, not in the contract):
  `state set-target-policy`, `silver trigger`, `silver reset`,
  `pelt fingerprint`, `fang open`, `fang open-profile`, `fang close`,
  `fang cleanup`. The rc.1 contract says "Hidden legacy commands are not
  supported v1 administration". Which ones the integration suites
  still use is not yet inventoried.
- **Tools.** `cargo-audit` is not installed on DarkSide now; Stage21 used
  it with a dated RustSec database.
- **Compatibility.** The first genesis moves the manifest to schema 2,
  which rc.1 binaries refuse (Stage28 S2), and applying a genesis is
  irreversible. A Den that never applied governance stays schema 1.

## 1. What "architecture complete" means

- **1a (recommended).** Every plane of ARCHITECTURE §55 has an
  implemented CLI path with a lab gate and a record; every remaining "future" item of
  ARCHITECTURE (KAD `STORE`, record relaying, automatic route selection,
  more than one relay, pairing, GUI/mobile, other platforms) is listed
  explicitly as outside this architecture-complete scope; and the
  contracts, operator documents and KNOWN_LIMITATIONS describe exactly
  what the build does. Complete means "what is claimed is built, certified
  and documented", not "the final vision".
- **1b.** As 1a, but one or more of the future items is required first
  (each one is its own stage).

## 2. Form and name of the candidate

- **2a (recommended).** A frozen, audited candidate like Stage21: a
  release-input commit, a reproducible Linux x86_64 archive with checksums
  and SBOM, a tag, and an audit record. Private, like everything here:
  nothing is published. Name: **`1.0.0-rc.2`**. 1.0.0 was never final,
  and rc.2 supersedes rc.1 as the candidate for it; rc.1 stays frozen as
  it is.
- **2b.** The same, named `1.1.0-rc.1`, reading rc.1 as the 1.0 line and
  the new stages as additions.
- **2c.** No archive or tag: an audit record on a commit of `main` only.

The Cargo package versions: **2-i (recommended)** align them with the
release version in the candidate commit, ending the documented
distinction; **2-ii** keep `0.1.0` and keep documenting it.

## 3. The CLI contract

- **3a (recommended).** Rewrite `V1_CLI_CONTRACT.md` as one consolidated
  contract for the candidate. Group the commands by plane, keep one table
  of exit classes (64, 65, 66, 69, 70 and the daemon's 67) and of the
  stable `--json` fields, and keep the per-stage history in a short
  appendix. Behaviour is unchanged; only the document changes. Within the
  candidate line, a command may be added but not removed or changed
  incompatibly.
- **3b.** Keep the appended sections and only fix the preamble.

The wire contract follows the same choice: add the Stage26 routed OPEN
and the Stage29 governance RPC, summarised from their specs, so the
contract covers every message the candidate sends.

## 4. Hidden commands

- **4a (recommended).** Keep them hidden and unsupported, as rc.1 says,
  and list them by name in the contract as "test-only, not
  administration, may change". No code change.
- **4b.** Inventory them, remove the ones no suite uses and move the rest behind the
  `lab-faults` build. This is a code change: separate PR, after this
  survey, before the freeze.

## 5. Documents before the freeze

- **5a (recommended).** Bring every document listed above up to the
  candidate build in one docs-only pass before the audit:
  - QUICKSTART and README gain the post-rc.1 workflows;
  - OPERATIONS and DAILY_COMMANDS gain services, discovery, routes,
    governance (including sync and auto-sync) and retirement;
  - the threat model and security properties gain the Stage24–31
    properties, each taken from its stage's security notes;
  - TRANSPORT_ARCHITECTURE gains the relay;
  - ARCHITECTURE's stale progress sections (§34, §44–46, §65) are
    corrected and the Stage31 behaviour is added;
  - KNOWN_LIMITATIONS becomes one list for the candidate, with the rc.1
    list kept as history.

  Docs only. No document claims more than a record shows.
- **5b.** Only the contracts and KNOWN_LIMITATIONS; the operator
  documents later.

## 6. Gates

- **6a (recommended).** The Stage21 set on the candidate archive:
  - advisories (`cargo-audit` with a dated database, which needs the
    owner to install it), dependency graph, SBOM, licenses and third-party
    notices;
  - `fmt`, `clippy -D warnings`, `cargo test --locked --offline`;
  - every Python integration suite and lab gate (service, discovery,
    service discovery, routed, provider, governance, activation, sync,
    death, auto-sync);
  - the Stage20 CI chaos;
  - archive install, uninstall and reinstall, and QUICKSTART followed as
    written, all from extracted archive binaries.

  Plus one **WAN smoke** of the candidate archive with disposable daemons:
  one open per transport, direct and routed, one discovery, one governance
  sync. The Stage22R and Stage32 campaigns are not rerun; their records
  stay the evidence, tied to the commits they ran on.
- **6b.** 6a without the WAN smoke.
- **6c.** 6a plus a full rerun of Stage22R D01–D09 on the candidate.

## 7. Upgrade from 1.0.0-rc.1

- **7a (recommended).** An upgrade test from the frozen rc.1 binaries:
  - a schema-1 Den populated by rc.1 starts under the candidate with its
    Pelt, Pack, targets, Silver and Fangs intact;
  - the candidate then applies a genesis and moves the manifest to
    schema 2;
  - rc.1 binaries refuse that Den cleanly, as documented, without
    damaging it.

  The rc.1 binaries come from the archive in `releases/`, which is never
  touched: the owner copies the archive out, or allows a read-only
  extraction into the scratchpad.
- **7b.** No downgrade check, only the upgrade.

## 8. Out of scope

Recommended out of scope for this candidate, and listed as such in
KNOWN_LIMITATIONS:

- Windows, macOS, ARM, other libc;
- GUI, mobile, pairing or remote administration;
- KAD `STORE`, record relaying, route discovery, more than one relay;
- automatic provider reputation;
- real host clock changes, suspend/resume;
- the D09 indeterminate commit on real storage (unit-tested);
- media-level erasure claims (Stage30);
- public release.

## Security notes

- The candidate changes no protocol, trust, handshake, KDF, replay or
  fallback behaviour. Any code change the decisions lead to (2-i, 4b) is
  a separate PR before the freeze, under the usual review.
- Documents are corrected to match the code and the records, never the
  other way: a gap between them found during step 5 is reported to the
  owner, not fixed silently in either.
- The audit result for advisories is dated, as in Stage21, not a claim
  about future advisories.

## Implementation (after decisions)

| Step | Scope |
|---|---|
| 1 | This survey; decisions |
| 2 | Code changes the decisions require (2-i, 4b), each its own PR |
| 3 | Documents (5a), docs only |
| 4 | Release inputs, archive, gates (6), upgrade test (7); audit record `STAGE33_CANDIDATE_AUDIT.md` |
| 5 | Tag and freeze, on the owner's word |

## Recommendations in one line

1a, 2a (`1.0.0-rc.2`) with 2-i, 3a, 4a, 5a, 6a, 7a, and section 8 as
stated.
