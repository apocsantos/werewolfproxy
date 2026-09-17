# WerewolfProxy v1 RC Linux service contract

Certification is limited to Linux x86_64, x86_64-unknown-linux-gnu, tested on
Debian 13.6 with glibc 2.41. The installed daemon remains in the foreground
under systemd Type=simple as User=werewolf, Group=werewolf. It needs no Linux
capability for its configured high ports, filesystem, DNS, outbound connect or
local Unix control socket.

The vendor unit uses StateDirectory=werewolfproxy (persistent /var/lib state),
RuntimeDirectory=werewolfproxy (recreated under /run), UMask=0077,
LimitNOFILE=4096, Restart=on-failure with delay and start limiting, and a
10-second systemd stop timeout. It enables NoNewPrivileges, an empty capability
set, PrivateTmp, ProtectSystem=strict, ProtectHome, kernel/control-group
protections, set-id/realtime restrictions, LockPersonality,
MemoryDenyWriteExecute and native syscall architecture restriction. The
certified unit is packaging/systemd/werewolfd.service.

Startup validates Den security, acquires the Den lock, validates manifest and
protected generation, loads Pelt/TLS identity, starts local control, restores
configured profiles and binds required secure listeners before READY. Silver
LOCKED is a valid administrative state; control remains available and
forwarding is fenced. Missing Pelt leaves secure listeners waiting and does not
generate identity. Manifest inconsistency fails closed at startup.

SIGTERM and foreground SIGINT use the same bounded owned-task shutdown. SIGHUP
does not reload configuration. SIGKILL cannot run cleanup; OS lock release,
durable state validation and safe stale socket recovery enable restart. A
second live process cannot take the same Den lock. Ordinary network, resolver
or target outage does not require daemon restart. Stage17 reports its exact
process, signal and restart tests; Stage21 makes no additional claim of a
real-host reboot certification.

Installation requires root to write system paths/create the service account.
The daemon runs unprivileged. The installer does not enable/start the service,
initialize Pelt or create trust. Administrator drop-ins are separate from the
vendor unit. User-mode foreground operation with an explicit private --home
remains supported without systemd.
