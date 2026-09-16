# Stage19 release harness

`release.py` inspects a generated Stage19 archive and installs it into a
disposable `DESTDIR`. It verifies the archive checksum manifest, exact
distributable inventory, idempotent installation, persistent-Den preservation,
default uninstall behavior, safe staging-root rejection, checksum failure
before replacement, and execution outside the source checkout.

It accepts only a locally generated archive and uses no network resources:

```sh
python3 -B tests/stage19_release/release.py --archive /tmp/werewolfproxy-1.0.0-rc.1-linux-x86_64.tar.gz
```

The harness never provisions a Pelt or a Pack and writes no cryptographic
material. Its sentinel is an inert, synthetic staging-state marker used only
to prove that installer and uninstaller operations do not modify persistent
state.

`upgrade.py` uses exact Stage18 and Stage19 binaries in a disposable Den. It
records an active-Fang intent and Silver lock under Stage18, atomically swaps
the binary pair to Stage19, verifies byte-for-byte state preservation, then
swaps back to Stage18 and re-establishes forwarding. It proves software
compatibility only; it does not make a persistent-state freshness claim.
