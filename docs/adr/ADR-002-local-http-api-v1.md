# ADR-002: local http api v1

Status: Accepted (H-075)

## Decision

Local HTTP API v1 binds loopback and requires a 256-bit bearer token on every
`/v1` route. `/healthz` is open and minimal. Foreign Host and browser-origin
requests are rejected. Provider credentials are separate.

## Context

Unauthenticated loopback access let any local process call execution and
approval routes. The execution request currently accepts a caller-selected
interpreter, making this a credential exposure risk until H-076 removes it.
DNS rebinding could also read local API data without a Host check.

## Consequences

The CLI reads `daemon.token` from the private data directory and sends it in
an Authorization header. The native desktop keeps using the CLI stdin bridge.
