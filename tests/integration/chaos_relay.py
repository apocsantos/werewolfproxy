#!/usr/bin/env python3
"""Stage32 raw TCP relay, for calibration only, run inside a node's network
namespace.

    chaos_relay.py READY_DIR LISTEN_HOST TARGET_HOST:PORT

Accepts TCP connections on an ephemeral port of LISTEN_HOST and relays each
to TARGET over a second TCP connection, both ways, with no timeout: two
kernel TCP legs, as a routed Fang's TCP hop has, and no Werewolf. Writes
{"relay": port} to READY_DIR/ready.json.
"""
import json
import pathlib
import socket
import sys
import threading


def pump(source, sink):
    try:
        while data := source.recv(65536):
            sink.sendall(data)
        sink.shutdown(socket.SHUT_WR)
    except OSError:
        pass


def main():
    directory, host, target = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
    target_host, target_port = target.rsplit(':', 1)
    listener = socket.socket()
    listener.bind((host, 0))
    listener.listen()
    pending = directory / 'ready.pending'
    pending.write_text(json.dumps({'relay': listener.getsockname()[1]}))
    pending.replace(directory / 'ready.json')
    while True:
        client, _ = listener.accept()
        upstream = socket.create_connection((target_host, int(target_port)))
        for source, sink in ((client, upstream), (upstream, client)):
            threading.Thread(target=pump, args=(source, sink), daemon=True).start()


if __name__ == '__main__':
    main()
