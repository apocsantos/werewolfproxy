#!/usr/bin/env python3
"""Stage39 M3: the D07 target (Stage22R D07, in the sandbox), on D.

    d07_target.py DIR    writes DIR/ready.json {"port": p, "absent": q}

One child process per connection. The client first sends `D07 <mode> <n>\n`.
The child echoes; for every mode but `echo`, after echoing exactly n bytes:
  rst   SO_LINGER 0 and close (abortive close, RST)
  kill  SIGKILLs itself (its kernel closes the socket: RST if unread data is
        queued, else FIN)
  fin   shutdown(SHUT_WR), then reads and discards until the client's EOF
`echo` echoes everything and closes after the client's EOF. `absent` is a
port on which nothing listens.
"""
import json
import os
import signal
import socket
import struct
import sys


def header(conn):
    line = b''
    while not line.endswith(b'\n'):
        b = conn.recv(1)
        if not b:
            raise EOFError
        line += b
    _, mode, n = line.decode().split()
    return mode, int(n)


def child(conn):
    mode, n = header(conn)
    echoed = 0
    while True:
        limit = 65536 if mode == 'echo' else min(65536, n - echoed)
        data = conn.recv(limit) if limit > 0 else b''
        if limit > 0 and not data:
            break
        conn.sendall(data)
        echoed += len(data)
        if mode != 'echo' and echoed >= n:
            if mode == 'rst':
                conn.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack('ii', 1, 0))
                conn.close()
            elif mode == 'kill':
                os.kill(os.getpid(), signal.SIGKILL)
            elif mode == 'fin':
                conn.shutdown(socket.SHUT_WR)
                while conn.recv(65536):
                    pass
                conn.close()
            return
    conn.close()


def main():
    signal.signal(signal.SIGCHLD, signal.SIG_IGN)
    listener = socket.socket()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(('127.0.0.1', 0))
    listener.listen(16)
    probe = socket.socket()
    probe.bind(('127.0.0.1', 0))
    absent = probe.getsockname()[1]
    probe.close()
    tmp = os.path.join(sys.argv[1], 'ready.json.tmp')
    with open(tmp, 'w') as f:
        json.dump({'port': listener.getsockname()[1], 'absent': absent}, f)
    os.replace(tmp, os.path.join(sys.argv[1], 'ready.json'))
    while True:
        conn, _ = listener.accept()
        if os.fork() == 0:
            listener.close()
            try:
                child(conn)
            except (OSError, EOFError):
                pass
            os._exit(0)
        conn.close()


if __name__ == '__main__':
    main()
