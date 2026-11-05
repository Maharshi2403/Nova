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
