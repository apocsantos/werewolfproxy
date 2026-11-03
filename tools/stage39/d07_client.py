#!/usr/bin/env python3
"""Stage39 M3: the D07 client, on A.

    d07_client.py ADDRESS MODE N TOTAL     prints one JSON line

Sends `D07 MODE N\n`, then TOTAL random bytes while reading concurrently,
then shuts down its send direction. Records how receive and send each ended
(eof/complete, reset, epipe, timeout, other), the bytes received, and
whether they are an exact prefix of the bytes sent. A reset collected by
either direction counts as the session's reset (D06/D07).
"""
import errno
import json
import os
import socket
import sys
import threading
import time

LIMIT_S = 180


def kind(error):
    return {errno.ECONNRESET: 'reset', errno.EPIPE: 'epipe'}.get(error.errno, 'other:' + str(error.errno))


def main():
    address, mode, n, total = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
    host, port = address.rsplit(':', 1)
    payload = os.urandom(total)
    started = time.monotonic()
    result = {'mode': mode, 'n': n, 'total': total}
    try:
        s = socket.create_connection((host, int(port)), timeout=30)
    except OSError as error:
        print(json.dumps({**result, 'connect': kind(error)}))
        return
    s.settimeout(LIMIT_S)
    received = bytearray()
    ends = {}

    def send():
        try:
            s.sendall(f'D07 {mode} {n}\n'.encode())
            s.sendall(payload)
            s.shutdown(socket.SHUT_WR)
            ends['send'] = 'complete'
        except socket.timeout:
            ends['send'] = 'timeout'
        except OSError as error:
            ends['send'] = kind(error)

    sender = threading.Thread(target=send)
    sender.start()
    try:
        while True:
            data = s.recv(1 << 16)
            if not data:
                ends['recv'] = 'eof'
                break
            received += data
    except socket.timeout:
        ends['recv'] = 'timeout'
    except OSError as error:
        ends['recv'] = kind(error)
    sender.join(LIMIT_S)
    s.close()
    result.update({
        'recv_end': ends.get('recv'), 'send_end': ends.get('send', 'unfinished'),
        'received': len(received), 'prefix_exact': bytes(received) == payload[:len(received)],
        'reset_seen': 'reset' in (ends.get('recv'), ends.get('send')),
        'seconds': round(time.monotonic() - started, 3)})
    print(json.dumps(result))


if __name__ == '__main__':
    main()
