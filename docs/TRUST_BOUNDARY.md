# Execution trust boundary

Registration defaults to untrusted and does not run Git or repository programs.
`harness init <repo> --name <name> --trust-local` explicitly authorizes trusted
local execution; Git registration then requires a clean checkout and an initial
commit. Existing projects without a trust record remain untrusted after migration.
Re-register a trusted project to make the decision explicit.

This version has a sandbox adapter interface, but ships no hard isolation backend.
Untrusted projects therefore cannot execute. A future backend must configure the
actual subprocess and isolate filesystem access, network and process trees.

Worktrees isolate branches, not operating-system privileges. Trusted-local tests
and generated code run with the user's privileges. Path checks, atomic writes and
environment scrubbing reduce accidental exposure but cannot stop malicious trusted
code or parent-directory TOCTOU attacks. Do not trust an unknown repository just to
bypass this gate. Approved commands still require exact argv authorization.
