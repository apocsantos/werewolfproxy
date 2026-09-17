# Two-node quickstart

This walkthrough uses two Linux x86_64 hosts and the final binary archive. It
uses user-mode foreground daemons so each node can bind its actual reachable
address. System-service installation is in packaging/INSTALL.md. Replace the
example addresses and ports with values reachable through your firewall.

On each host, extract the archive, verify its checksum, and install it. The
installer writes system paths but does not enable the service or create an
identity. For this walk-through, use the installed binaries directly with
explicit private Den and runtime paths:

    tar -xzf werewolfproxy-1.0.0-rc.1-linux-x86_64.tar.gz
    cd werewolfproxy-1.0.0-rc.1-linux-x86_64
    sha256sum -c SHA256SUMS
    sudo ./install.sh
    mkdir -m 700 -p "$HOME/.config/werewolf-a" "$HOME/.local/run/werewolf-a"
    werewolfctl --home "$HOME/.config/werewolf-a" den init

Use a different suffix, such as werewolf-b, on node B. Start werewolfd in a
terminal on each node, substituting its externally reachable host address:

    werewolfd --home "$HOME/.config/werewolf-a" --socket "$HOME/.local/run/werewolf-a/control.sock" --listen 0.0.0.0:8443 --quic-listen 0.0.0.0:9560

Use the same command on B with its own paths. The control socket is placed in
the private runtime directory; the daemon remains in the foreground. Restrict
the selected ports at the host firewall according to the deployment.

In another terminal on both nodes, initialize identity, explicitly migrate
the initial protected state, and enable operation by opening Silver:

    werewolfctl --socket "$HOME/.local/run/werewolf-a/control.sock" pelt init
    werewolfctl --socket "$HOME/.local/run/werewolf-a/control.sock" state manifest-migrate
    werewolfctl --socket "$HOME/.local/run/werewolf-a/control.sock" silver off
    werewolfctl --socket "$HOME/.local/run/werewolf-a/control.sock" --json pelt show

Use node B's equivalent socket on B. Exchange only public_key_b64 and
fingerprint from the two pelt show results through a trusted out-of-band
channel. These values are public. Never exchange pelt.json or private key
material.

On A add B's public key and reachable secure TCP listener. On B add A's key
and address:

    werewolfctl --socket "$HOME/.local/run/werewolf-a/control.sock" pack add node-b BASE64_B_PUBLIC_KEY tcp://B_ADDRESS:8443
    werewolfctl --socket "$HOME/.local/run/werewolf-b/control.sock" pack add node-a BASE64_A_PUBLIC_KEY tcp://A_ADDRESS:8443

Assume a target application is already listening on node B at
127.0.0.1:7000. On B, allow that exact target for A's Pelt fingerprint. On A,
create and activate a local forwarding Fang:

    werewolfctl --socket "$HOME/.local/run/werewolf-b/control.sock" target allow A_FINGERPRINT 127.0.0.1:7000
    werewolfctl --socket "$HOME/.local/run/werewolf-a/control.sock" fang create example node-b 127.0.0.1:17000 127.0.0.1:7000 --transport tcp
    werewolfctl --socket "$HOME/.local/run/werewolf-a/control.sock" fang activate example
    werewolfctl --socket "$HOME/.local/run/werewolf-a/control.sock" status
    werewolfctl --socket "$HOME/.local/run/werewolf-a/control.sock" doctor

Connect the local client application on A to 127.0.0.1:17000 and verify its
normal application exchange with the target on B. The Stage18 CLI harness
performs deterministic echo and 50 MiB integrity tests using disposable local
fixtures; the quickstart does not create or expose a test target service.

The receiving node must authorize the exact target independently. Pack
membership authenticates a peer identity; it does not grant unrestricted
target or resource access. The default secure selector tries QUIC, then
encrypted TCP, and fails closed. Plain transport is never selected implicitly.

To stop forwarding, deactivate the Fang. Use silver on to fence new forwarding
and silver off only after the administrative/storage condition is healthy. Use
pack remove to revoke a peer; the daemon installs a runtime denial fence
before persisting the change. To stop the foreground daemon, send SIGTERM or
press Ctrl-C. For backup and system-service operation, follow
packaging/INSTALL.md.
