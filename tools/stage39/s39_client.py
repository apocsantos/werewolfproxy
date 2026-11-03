#!/usr/bin/env python3
"""Stage39 M1 application client on A (diagnostic only).

    s39_client.py ADDRESS MODE LOG STOPFILE

Connects to the Fang's local listener and records JSON lines with
monotonic times: connect, a verified echo probe (idle), byte progress each
second, and the first EOF, reset or error. It keeps the connection open
until STOPFILE appears: it never closes because a wait ended, so session
persistence is not a client artefact. MODE: idle, upload or download.
"""
import errno
import json
import os
import select
import socket
import sys
import time


def main():
    address, mode, log_path, stop = sys.argv[1:5]
    host, port = address.rsplit(':', 1)
    log = open(log_path, 'a', buffering=1)
    emit = lambda ev, **kw: log.write(json.dumps({'t': time.monotonic(), 'wall': time.time(),
                                                  'ev': ev, **kw}) + '\n')
    s = socket.create_connection((host, int(port)), timeout=30)
    s.settimeout(None)
    emit('connected')
    sent = received = 0
    if mode == 'idle':
        probe = os.urandom(64)
        s.sendall(probe)
        got = b''
        while len(got) < len(probe):
            part = s.recv(len(probe) - len(got))
            if not part:
                break
            got += part
        emit('probe', ok=got == probe)
    elif mode == 'download':
        s.sendall(b'G')
    s.setblocking(False)
    chunk = b'\x5a' * 16384
    ended = None
    last_report = time.monotonic()
    while not os.path.exists(stop):
        want_write = mode == 'upload' and ended is None
        r, w, _ = select.select([s] if ended is None else [], [s] if want_write else [], [], 0.2)
        try:
            if r:
                data = s.recv(65536)
                if not data:
                    ended = 'eof'
                    emit('eof', sent=sent, received=received)
                else:
                    received += len(data)
            if w:
                sent += s.send(chunk)
        except OSError as error:
            if error.errno in (errno.EAGAIN, errno.EWOULDBLOCK):
                continue
            ended = errno.errorcode.get(error.errno, str(error.errno))
            emit('error', kind=ended, sent=sent, received=received)
        now = time.monotonic()
        if now - last_report >= 1.0:
            emit('progress', sent=sent, received=received)
            last_report = now
    emit('stop', sent=sent, received=received, ended=ended)
    s.close()
    emit('closed')


if __name__ == '__main__':
    main()
