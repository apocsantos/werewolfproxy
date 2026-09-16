#!/usr/bin/env python3
"""Private HTTP fixtures and a binary TCP echo target; no external dependencies."""
import functools
import http.server
import json
import pathlib
import socketserver
import sys
import threading

MARKER = b'WerewolfProxy checkout Stage 0 application payload\n'
LARGE_SHA256 = '80d160f35ab0b90c95b2f4777c0fa0127bb2a4b5f6fff2f4a6c0f3a4c1219ea6'


class Echo(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(10)
        while data := self.request.recv(65536):
            self.request.sendall(data)


class TCPServer(socketserver.ThreadingTCPServer):
    daemon_threads = True


def main():
    directory = pathlib.Path(sys.argv[1])
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(directory))
    servers = [http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler),
               TCPServer(('127.0.0.1', 0), Echo)]
    for server in servers:
        threading.Thread(target=server.serve_forever, daemon=True).start()
    ready = directory / 'ready.json'
    pending = directory / 'ready.pending'
    pending.write_text(json.dumps(dict(zip(('http', 'echo'),
                                          [s.server_address[1] for s in servers]))))
    pending.replace(ready)
    threading.Event().wait()


if __name__ == '__main__':
    main()
