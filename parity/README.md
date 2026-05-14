# parity/

Fixtures and snapshot inputs for the Python → Rust migration. Each Rust port of
a tool drops a fixture here that pins the Python output for the same input, so
the Rust version can be asserted byte- or schema-equal during the dual-run
window.

Layout per fixture:

```
parity/<tool_name>/
  ├── input.json        # args passed to the tool
  ├── expected.json     # captured Python output (truth oracle)
  └── notes.md          # any caveats (volatile fields, time-sensitive data)
```

Captures should be regenerated when the upstream site changes shape, not when
the Rust impl diverges — the whole point is to catch divergence.
