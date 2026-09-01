# Mac interop fixtures

Real frames captured from a running Mac Entanglo peer, committed as
`.bin` files (raw bytes, length prefix included) so a future protocol
change on either side fails a test on push instead of silently
drifting.

See `SKELETON.md` → "To capture Mac fixtures" for how to produce
these: attach lldb to the Mac app, or add a one-time logging branch in
`NetworkTransport.swift`, and dump the first `hello`, `heartbeat`, and
a double-click `inputEvent` frame here.

Empty today — add fixtures as part of Phase 1 interop testing.
