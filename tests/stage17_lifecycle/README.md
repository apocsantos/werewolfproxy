# Stage 17 lifecycle harness

`lifecycle.py` runs only against a disposable private Den. It verifies
foreground SIGTERM/SIGINT/SIGHUP behavior, duplicate-start exclusion, bind
failure cleanup, 50 clean restarts, 10 SIGKILL/stale-socket restarts, and a
malformed Stage15B selector. It creates no packet capture or retained secret
material.

Run it after building both binaries:

```sh
python3 -B tests/stage17_lifecycle/lifecycle.py \
  --daemon target/debug/werewolfd \
  --control target/debug/werewolfctl \
  --tmp-parent "$HOME/werewolf-stage17-tmp"
```

The parent must be a mode-0700 directory beneath the invoking user's secure
home. A shared `/tmp` ancestry intentionally fails the Stage11B Den checks.
