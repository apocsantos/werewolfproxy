#!/usr/bin/env python3
"""Stage32 transfer client, run inside a node's network namespace.

    chaos_client.py duplex HOST:PORT BYTES LABEL TIMEOUT_S

Sends BYTES of deterministic data (derived from LABEL) while reading the
echo, simultaneously, in one non-blocking loop, and prints one JSON line:
bytes sent and received, the SHA-256 of each direction, the elapsed
seconds, the times to the first and the last echoed byte, whether every
echoed byte equals the data sent at its offset, and how the transfer ended.
The only client-side deadline is TIMEOUT_S, the outer bound: nothing else
in the client can end a transfer, so an early end is the connection's.
"""
import hashlib
import json
import selectors
import socket
import sys
import time


def duplex(address, total, label, limit):
    host, port = address.rsplit(':', 1)
    block = hashlib.sha256(label.encode()).digest() * 2048  # 64 KiB
    sent, received = hashlib.sha256(), hashlib.sha256()
    n_sent = n_received = 0
    first_echo = last_echo = None
    in_order = True
    error = None
    started = time.monotonic()
    deadline = started + limit
    conn = socket.socket()
    conn.setblocking(False)
    selector = selectors.DefaultSelector()
    try:
        try:
            conn.connect((host, int(port)))
        except BlockingIOError:
            pass
        selector.register(conn, selectors.EVENT_READ | selectors.EVENT_WRITE)
        write_open = True
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                error = 'outer bound exceeded'
                break
            events = selector.select(timeout=min(left, 1))
            done = False
            for _, mask in events:
                if mask & selectors.EVENT_WRITE and write_open:
                    if n_sent < total:
                        offset = n_sent % len(block)
                        chunk = block[offset:offset + min(len(block) - offset, total - n_sent)]
                        try:
                            count = conn.send(chunk)
                        except BlockingIOError:
                            count = 0
                        sent.update(chunk[:count])
                        n_sent += count
                    if n_sent == total:
                        conn.shutdown(socket.SHUT_WR)
                        write_open = False
                        selector.modify(conn, selectors.EVENT_READ)
                if mask & selectors.EVENT_READ:
                    try:
                        data = conn.recv(65536)
                    except BlockingIOError:
                        continue
                    if not data:
                        done = True
                        break
                    last_echo = time.monotonic() - started
                    if first_echo is None:
                        first_echo = last_echo
                    offset = n_received % len(block)
                    expected = (block[offset:] + block * (len(data) // len(block) + 1))[:len(data)]
                    in_order = in_order and data == expected
                    received.update(data)
                    n_received += len(data)
            if done:
                break
    except OSError as failure:
        error = f'{type(failure).__name__}: {failure}'
    finally:
        selector.close()
        conn.close()
    elapsed = time.monotonic() - started
    complete = n_sent == total and n_received == total
    if error is None and not complete:
        error = 'closed early'
    return {'bytes': total, 'sent': n_sent, 'received': n_received,
            'sent_sha256': sent.hexdigest(), 'received_sha256': received.hexdigest(),
            'match': complete and sent.digest() == received.digest(),
            'complete': complete, 'elapsed_s': round(elapsed, 3),
            'first_echo_s': None if first_echo is None else round(first_echo, 3),
            'last_echo_s': None if last_echo is None else round(last_echo, 3),
            'echo_in_order': in_order,
            'error': error}


def main():
    mode, address, total, label, limit = sys.argv[1:6]
    if mode != 'duplex':
        sys.exit(f'unknown mode {mode}')
    print(json.dumps(duplex(address, int(total), label, float(limit))), flush=True)


if __name__ == '__main__':
    main()
