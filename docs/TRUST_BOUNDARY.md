# Execution trust boundary

Registration defaults to untrusted and does not run Git or repository programs.
`nova init <repo> --name <name> --trust-local` explicitly authorizes trusted
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

## Threats and controls

- Repository prompt injection is untrusted data. It cannot approve a plan, alter
  Rust budgets, change task state or authorize a tool. A worker completion claim
  never replaces external verification and integration evidence.
- Traversal, protected paths, symlink escapes and Windows path aliases are denied.
  Writes use atomic replacement to avoid overwriting an external hardlink inode.
  Parent-directory races remain a trusted-local limitation.
- Risky commands require exact scoped single-use human approvals. Test argv is
  explicitly reviewed at execution launch; a test runner can execute arbitrary repo
  code and must not be treated as intrinsically safe.
- Coder workers have scrubbed environments and isolated imports. Only a separate
  provider adapter gets only the selected saved credential for one authorized bounded request.
  Rust records provider usage; uncertain calls retain reserved spend.
- The local API has no authentication for now. It binds only loopback and rejects
  browser-origin/cross-site requests. Other local processes can control it; it is
  not a multi-user boundary. Provider API credentials still use the configured vault.
- Logs/artifacts are bounded and redact known credentials/token patterns. This is
  not a guarantee of detecting every unknown secret. Do not place secrets in prompts,
  test argv or source files supplied to a model.
- Crashes retain worktrees, reconcile leases and block uncertain side effects.
  Running verifier commands can outlive a forcibly killed daemon. The runtime does
  not identify arbitrary saved PIDs as safe to kill, and Windows taskkill is not a
  Job Object isolation guarantee.

Database rows and retained worktrees can contain private project information.
Backups need the same access protection as source code. There is no remote/multi-user
security boundary, automatic secret-store integration, disk encryption guarantee,
or shipped OS sandbox backend. Desktop UI and automatic push/deploy are deferred.
