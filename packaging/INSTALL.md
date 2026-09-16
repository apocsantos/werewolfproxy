# WerewolfProxy Linux installation

This archive is certified only for Linux x86_64 with a systemd service manager.
Verify `SHA256SUMS` before installation:

```sh
sha256sum -c SHA256SUMS
```

Install software as root. The installer creates `/var/lib/werewolfproxy` with
mode `0700` for the unprivileged `werewolf` service account, but does not
create a Pelt or trust any peer:

```sh
./install.sh
```

It installs `werewolfd` and `werewolfctl` in `/usr/local/bin` and the service unit in
`/usr/lib/systemd/system`. It does not enable or start the service. Provision
identity and trust explicitly, then opt into service activation:

```sh
sudo -u werewolf werewolfctl --home /var/lib/werewolfproxy den init
systemctl start werewolfd
sudo -u werewolf werewolfctl --socket /run/werewolfproxy/control.sock pelt init
sudo -u werewolf werewolfctl --socket /run/werewolfproxy/control.sock state manifest-migrate
sudo -u werewolf werewolfctl --socket /run/werewolfproxy/control.sock silver off
systemctl enable --now werewolfd
```

The local control socket authorizes the daemon's Unix uid, so service
administration uses `sudo -u werewolf`. Use `werewolfctl pelt show` for public peer provisioning material. Do not put
private keys in shell history. See the Stage18 operator guide for Pack, target,
Fang, and Silver commands.

Before an upgrade, stop the service and make a permission-preserving complete
Den backup to a private administrator-controlled location. The backup includes
Pelt private material:

```sh
systemctl stop werewolfd
install -d -m 0700 /root/werewolf-backup
cp -a /var/lib/werewolfproxy /root/werewolf-backup/
./install.sh
systemctl start werewolfd
sudo -u werewolf werewolfctl --socket /run/werewolfproxy/control.sock status
sudo -u werewolf werewolfctl --home /var/lib/werewolfproxy doctor
```

An upgrade intentionally ends active forwarding sessions; clients reconnect.
To roll back software, stop the service and install a previously verified
Stage18/Stage19-compatible archive. This is software rollback, not protection
against persistent-state rollback. Complete historical Dens remain subject to
the Stage15B external-anchor nonclaim.

`./uninstall.sh` removes software only. It retains `/var/lib/werewolfproxy`,
including Pelt and all policy state. Stop/disable the service first. User-mode
operation remains supported without the system service:

```sh
werewolfd --home "$HOME/.config/werewolf" --socket "$XDG_RUNTIME_DIR/werewolf/control.sock"
werewolfctl --socket "$XDG_RUNTIME_DIR/werewolf/control.sock" status
```
