#!/usr/bin/env python3
"""Stage39 M1 test target on D (diagnostic only): three TCP services on
127.0.0.1, with no idle timeout.

  echo    echoes everything until EOF
  sink    reads and discards (upload tests)
  source  after the first byte from the client, writes 16 KiB chunks forever
          (download tests)

    s39_targets.py DIR      writes DIR/ready.json {"echo": p, "sink": p, "source": p}
"""
import json
import os
import socket
import sys
import threading


def serve(listener, mode):
    while True:
        conn, _ = listener.accept()
        threading.Thread(target=handle, args=(conn, mode), daemon=True).start()


def handle(conn, mode):
    try:
        if mode == 'echo':
            while True:
                data = conn.recv(65536)
                if not data:
                    break
                conn.sendall(data)
        elif mode == 'sink':
            while conn.recv(65536):
                pass
        else:
            if conn.recv(1):
                chunk = b'\xa5' * 16384
                while True:
                    conn.sendall(chunk)
    except OSError:
        pass
    finally:
        conn.close()


def main():
    directory = sys.argv[1]
    ports = {}
    for mode in ('echo', 'sink', 'source'):
        listener = socket.socket()
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(('127.0.0.1', 0))
        listener.listen(16)
        ports[mode] = listener.getsockname()[1]
        threading.Thread(target=serve, args=(listener, mode), daemon=True).start()
    tmp = os.path.join(directory, 'ready.json.tmp')
    with open(tmp, 'w') as f:
        json.dump(ports, f)
    os.replace(tmp, os.path.join(directory, 'ready.json'))
    threading.Event().wait()


if __name__ == '__main__':
    main()
