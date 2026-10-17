# Archive

Historical material kept for provenance. Nothing here is part of the
1.0.0-rc.1 build, release archive, or certified behavior.

## legacy-patches/

Pre-Stage9 shell scripts (`bootstrap_werewolf.sh`, `patch_*.sh`) that
bootstrapped the early workspace and applied features incrementally before
the codebase moved to staged branches. They are superseded by the source
tree and git history and should not be run against the current tree.

## support-bundles/

Support and log tarballs from May 2026 referenced as historical evidence in
[BASELINE.md](../BASELINE.md):

- `werewolf_support_20260526_*.tar.gz`: older QUIC diagnostics, Pack/profile
  samples and restoration logs with different identities. They contain
  public keys and loopback addresses only, no private key material.
- `werewolfproxy_authenticated_fang_v1.tar.gz` and
  `werewolfproxy_rc1_silver_logs_20260524_1735.tar.gz`: empty archives.
