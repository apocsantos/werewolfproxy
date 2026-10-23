#!/usr/bin/env python3
"""Disposable checkout-only provider selection gate (Stage27). See README.md.

Wolf A's service `web` lists providers B, C and D in that order. Each
provider's target first sends a tag naming it, so every connection shows
which provider served it. The gate checks that a provider is skipped only
when it fails before its session is established (no grant, unreachable),
never after; that each provider still authorizes A itself; that all failing
resets the local connection within the selection window; and that a Pack
change to any listed provider closes the Fang.
"""
import socket
import struct
import sys
import threading
import time

sys.dont_write_bytecode = True
from run import main, require, write_json
from service_gate import REJECTED, ServiceGate


class TaggedTarget:
    """Sends `<tag>\\n`, then echoes; `die` makes it close with a reset."""

    def __init__(self, tag):
        self.tag, self.connections = tag, 0
        self.listener = socket.create_server(('127.0.0.1', 0))
        self.address = f'127.0.0.1:{self.listener.getsockname()[1]}'
        threading.Thread(target=self.serve, daemon=True).start()

    def serve(self):
        while True:
            try:
                conn, _ = self.listener.accept()
            except OSError:
                return
            self.connections += 1
            threading.Thread(target=self.answer, args=(conn,), daemon=True).start()

    def answer(self, conn):
        try:
            conn.sendall(f'{self.tag}\n'.encode())
            while data := conn.recv(65536):
                if data.startswith(b'die'):
                    conn.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack('ii', 1, 0))
                    break
                conn.sendall(data)
        except OSError:
            pass
        finally:
            conn.close()


class ProviderGate(ServiceGate):
    WOLVES = ('a', 'b', 'c', 'd')

    def connect_web(self, *extra, expect=0):
        self.release('web')
        result = self.ctl('a', 'service', 'connect', 'web', '--local-port', self.ports['web'],
                          *extra, expect=expect)
        if expect == 0:
            self.wait_for(lambda: self.listener('web'), 'web listener')
        return result.get('result', {})

    def close_all(self):
        for fang in self.active_fangs('a'):
            self.ctl('a', 'fang', 'close', fang['id'])

    def served_by(self):
        """One session: which provider's tag arrives, and an exact echo."""
        started = time.monotonic()
        with socket.create_connection(('127.0.0.1', self.ports['web']), timeout=25) as conn:
            reader = conn.makefile('rb')
            try:
                tag = reader.readline().decode().strip()
            except (ConnectionResetError, socket.timeout) as exc:
                return {'end': type(exc).__name__, 'secs': round(time.monotonic() - started, 2)}
            if not tag:
                return {'end': 'eof', 'secs': round(time.monotonic() - started, 2)}
            conn.sendall(b'ping\n')
            echoed = reader.readline()
        require(echoed == b'ping\n', f'{tag}: echo mismatch {echoed!r}')
        return {'tag': tag, 'secs': round(time.monotonic() - started, 2)}

    def counts(self, targets):
        return {name: target.connections for name, target in targets.items()}

    def run(self):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        self.reserve('web')
        self.reserve('dead', udp=True)
        targets = {name: TaggedTarget(name) for name in ('b', 'c', 'd')}

        for wolf in self.WOLVES:
            (self.base / wolf).mkdir(mode=0o700)
            write_json(self.base / wolf / 'target_policy.json',
                       {'mode': 'deny-by-default', 'peers': {}})
        self.start_wolves()
        keys = {}
        for wolf in self.WOLVES:
            self.ctl(wolf, 'pelt', 'init')
            keys[wolf] = self.ctl(wolf, 'pelt', 'show')['result']['public_key_b64']
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP')
        for provider in ('b', 'c', 'd'):
            self.ctl('a', 'pack', 'add', f'wolf-{provider}', keys[provider],
                     f'quic://{self.address(provider + "_quic")}')
            self.ctl(provider, 'pack', 'add', 'wolf-a', keys['a'], f'quic://{self.address("a_quic")}')
        self.ctl('a', 'service', 'add', 'web', 'wolf-b', targets['b'].address, 'tcp')
        for provider in ('c', 'd'):
            self.ctl('a', 'service', 'provider', 'add', 'web', f'wolf-{provider}',
                     targets[provider].address)
        shown = self.ctl('a', 'service', 'show', 'web')['result']
        require([shown['peer']] + [p['peer'] for p in shown['fallbacks']]
                == ['wolf-b', 'wolf-c', 'wolf-d'], f'provider order: {shown}')

        # C and D grant A their targets; B grants nothing and refuses.
        for provider in ('c', 'd'):
            self.ctl(provider, 'target', 'allow', 'wolf-a', targets[provider].address)
        opened = self.connect_web()
        require(opened.get('providers') == ['wolf-b', 'wolf-c', 'wolf-d'], f'open: {opened}')
        fang = self.active_fangs('a')
        require(fang[0]['transport'] == 'providers: wolf-b, wolf-c, wolf-d', f'{fang}')
        refused = self.served_by()
        require(refused.get('tag') == 'c' and targets['b'].connections == 0,
                f'B refused, expected C: {refused} {self.counts(targets)}')
        self.record('a provider that refuses is skipped before any byte',
                    {'served_by': refused['tag'], 'connections': self.counts(targets)})

        # Once B grants, B serves; C and D see nothing more.
        self.ctl('b', 'target', 'allow', 'wolf-a', targets['b'].address)
        before = self.counts(targets)
        first = self.served_by()
        require(first.get('tag') == 'b', f'expected B: {first}')
        require(self.counts(targets) == {**before, 'b': before['b'] + 1},
                f'other providers were contacted: {self.counts(targets)}')
        self.record('the first provider that accepts serves', first)

        # After establishment, a failure is never retried elsewhere.
        before = self.counts(targets)
        with socket.create_connection(('127.0.0.1', self.ports['web']), timeout=20) as conn:
            reader = conn.makefile('rb')
            require(reader.readline() == b'b\n', 'B did not serve')
            conn.sendall(b'die\n')
            try:
                end = 'eof' if conn.recv(64) == b'' else 'data'
            except ConnectionResetError:
                end = 'reset'
        time.sleep(1)
        require(end in ('reset', 'eof'), f'session end: {end}')
        require(self.counts(targets) == {**before, 'b': before['b'] + 1},
                f'a failed session was retried: {self.counts(targets)}')
        self.record('a failure after establishment is not retried', {'end': end})

        # B becomes unreachable (its Pack address now points at a silent UDP
        # port). The address change closes the Fang; reopened, C serves after
        # B's handshake deadline, within the selection window.
        changed = self.ctl('a', 'pack', 'set-address', 'wolf-b',
                           f'quic://{self.address("dead")}')['result']
        require(changed['closed_fangs'] == 1 and self.active_fangs('a') == [],
                f'provider Fang not closed by a provider address change: {changed}')
        self.connect_web()
        down = self.served_by()
        require(down.get('tag') == 'c' and 3 < down['secs'] < 15, f'expected C after B: {down}')
        self.record('an unreachable provider is skipped within the window',
                    {**down, 'closed_on_address_change': changed['closed_fangs']})

        # Every provider failing resets the local connection within the window.
        for provider in ('c', 'd'):
            self.ctl(provider, 'target', 'remove', 'wolf-a', targets[provider].address)
        failed = self.served_by()
        require(failed.get('end') in ('ConnectionResetError', 'eof') and failed['secs'] < 16,
                f'all providers failing: {failed}')
        self.record('all providers failing resets the connection', failed)

        # Removing any listed provider from the Pack closes the Fang.
        self.ctl('a', 'pack', 'remove', 'wolf-d')
        require(self.active_fangs('a') == [], 'provider Fang survived a Pack removal')
        self.ctl('a', 'service', 'connect', 'web', '--local-port', self.ports['web'],
                 expect=REJECTED)
        self.record('a Pack change to any provider closes the Fang',
                    {'reopen_with_missing_provider_exit': REJECTED})
        self.ctl('a', 'service', 'provider', 'remove', 'web', 'wolf-d')
        self.connect_web('--transport', 'tcp-plain', expect=REJECTED)
        self.record('plain TCP is refused with fallbacks', {'exit': REJECTED})
        require(all(p.poll() is None for p in self.wolves), 'gate process exited')


if __name__ == '__main__':
    sys.exit(main(ProviderGate))
