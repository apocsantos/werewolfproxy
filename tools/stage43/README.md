# Stage43 tools

`fairness.py` reproduces the survey's scenarios A (idle connections fill a
Fang) and D (one source address fills a Fang bound to a non-loopback
address). See `docs/STAGE43_M1_FAIRNESS_REPRODUCTION.md`.

- It reuses `tools/stage42/fang_flood.py`, including its 50 MB log guard.
  Never remove the guard (Stage42 M1 §5).
- `source:` scenarios run in a user, mount and network namespace with a
  private root and a private 512 MB `/tmp`. Nothing is run as the host's
  root, and the lab never touches the host `/tmp` or any live wolf.
- `idle:` scenarios use their own echo target with no idle timeout; the
  lab's echo target closes connections that are idle for 10 s.
