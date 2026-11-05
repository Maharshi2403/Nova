# Remaining implementation scope

The supplied TASK_BACKLOG.md remains the governing ticket order. Inspection after
H-052 found that the original H-038 E2E bypassed CLI, worker processes and broker
writes. The daemon only advanced readiness. G4/G5 therefore cannot be considered
fully demonstrated by the earlier commits.

Before implementation, split H-053 into these bounded children:

- H-053a: connect a real supervised worker, broker, verifier and integrator through
  the bounded pool to an explicit authenticated run execution endpoint and CLI.
  Include offline execution evidence on a disposable repository. Depends H-049..052.
- H-053b: add deterministic 100-task load and fault/restart coverage for the
  connected runtime. Depends H-053a. This completes H-053's original test scope.

H-054..H-060 retain their original scope and depend on both H-053 children wherever
the original dependencies named H-053. Security and release gates require actual
Windows/macOS CI evidence; local macOS results alone never establish those gates.
Live provider requests remain opt-in; no paid API requests are authorized by this
implementation run. Existing adapter capabilities are tested with fake HTTP.

## Live execution gap discovered before release checks

H-043's original provider adapter was not connected to a coder or daemon dispatch.
Complete these bounded backfill tickets after H-057, before H-058 release checks:

- H-043b: structured live coder loop using the existing adapter, bounded prompt
  builder and retry policy; broker authorization before every provider attempt;
  fake-provider unit tests and protocol-only stdout. Depends H-043..046,H-056.
- H-043c: Rust-owned live model routing, conservative per-attempt spend reservation,
  usage pricing and explicit CLI opt-in, credential isolation and command broker
  wiring. Depends H-043b,H-057. No real provider calls during implementation.

H-058's full validation includes both backfill tickets. This is recorded before
implementation so an adapter-only build is not presented as a complete live harness.

## H-058a release-review fixes

Before documenting operator workflows, fix repeated pause/resume audit keys,
concurrent scheduler compare-and-set races, and per-run worktree-registry ownership.
These were found during H-059 workflow review; verify repeat control cycles and
keep each run's registry separate. No new product scope or remote release.
