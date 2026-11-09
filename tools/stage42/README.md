# Stage42 tools

`fang_flood.py`: Stage42 M1 reproduction (see
`docs/STAGE42_M1_FANG_FLOOD_REPRODUCTION.md`). It reuses the acceptance lab:
fresh Dens on /tmp, dynamic loopback ports, the checkout build, and only its
own process groups are stopped.

    python3 -B tools/stage42/fang_flood.py fd:quic fd:tcp fd:tcp-plain \
        quota:quic quota:tcp quota:tcp-plain --out target/stage42-m1/run.json
    python3 -B tools/stage42/fang_flood.py fd:quic fd:tcp --nofile 64 --out ...

A QUIC Fang that runs out of descriptors (on `main` before Stage42 M2) logs
every accept retry. The tool stops the lab daemons when any lab log passes
50 MB. Do not remove that guard: an unguarded run filled DarkSide's /tmp.
