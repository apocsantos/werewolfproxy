#!/usr/bin/env python3
"""Stage32 echo target, run inside a node's network namespace.

    chaos_echo.py READY_DIR [HOST]

Echoes each TCP connection back to itself with no idle timeout: only the
connection's own end, clean or not, ends it. (The targets.py echo closes a
connection after 10 s without data, which ended transfers that were only
stalled by impairment; spec, Step 3 notes, harness defect.) Listens on an
ephemeral port on HOST (default loopback) and writes {"echo": port} to
READY_DIR/ready.json.
"""
import json
import pathlib
import socketserver
import sys
import threading


class Echo(socketserver.BaseRequestHandler):
    def handle(self):
        try:
            while data := self.request.recv(65536):
                self.request.sendall(data)
        except OSError:
            pass


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True


def main():
    directory = pathlib.Path(sys.argv[1])
    host = sys.argv[2] if len(sys.argv) > 2 else '127.0.0.1'
    server = Server((host, 0), Echo)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    pending = directory / 'ready.pending'
    pending.write_text(json.dumps({'echo': server.server_address[1]}))
    pending.replace(directory / 'ready.json')
    threading.Event().wait()


if __name__ == '__main__':
    main()
