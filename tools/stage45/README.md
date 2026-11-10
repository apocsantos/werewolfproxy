# Stage45 tools

`visibility.py` runs the Stage45 scenarios: Fang sessions that fail, and
what the initiator logs and lists. See
`docs/STAGE45_M1_VISIBILITY_REPRODUCTION.md`.

- It builds on the acceptance lab (`tests/integration/run.py`,
  `service_gate.py`): fresh Dens, dynamic loopback ports, only its own
  process groups stopped, no live wolf touched.
- It keeps the Stage42 50 MB log guard. Never remove it: `flood:tcp-plain`
  exists to measure log growth (Stage42 M1 §5).
