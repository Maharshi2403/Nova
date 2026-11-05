# Native desktop dashboard

Nova's macOS desktop client is native SwiftUI. It uses a bounded JSON stdin bridge
to the Nova CLI; the CLI reads the daemon credential privately. No bearer token or
API key appears in an argument or URL. The daemon still rejects browser origins.
There is no hosted dashboard or telemetry. Live models receive approved task context.

The window shows projects, paged runs, task states, plan versions, model usage,
scoped command approvals, verifier results and task/integration commit identities.
Only durable verified/integrated state is presented as completed. Unknown costs
remain unknown. The activity view shows up to the first 1,000 events, with the last
100 of that page visible. CLI event cursors remain available for larger histories.

Provider setup uses a hidden key field, account-model discovery and macOS Keychain.
No generation request is made when connecting a provider. Unknown model metadata
still needs `nova models configure`; API keys alone cannot establish unknown prices
or structured-output support. Routes can be pinned or returned to complexity/risk
defaults in the Model routes view.

Add an existing committed Git repository with a name and explicit local trust choice.
A project can be added while it has uncommitted changes; import leaves those files
untouched. Execution requires a clean working tree. Commit or stash your work
before execution. Untrusted projects cannot execute. Tests run with your user
permissions; worktrees are not an OS sandbox.

The task-plan editor constructs an operator-authored plan without JSON. Enter a
high-level goal, then each task's title, description, allowed paths, use case, risk,
complexity, acceptance argv and spend cap. One argument per line means exact argv,
not shell syntax. A task can depend on the previous task or run independently.
Automatic model-based goal decomposition is not part of this editor.

Saving creates a proposed plan. Review the task scope and acceptance, then approve
that exact version. Execute is a separate action with a visible mode/command review.
Live mode uses saved providers and budgets. Mock mode writes synthetic fixture text
and belongs only in a disposable demo repository. Closing the window leaves Nova's
independent service running. Pause stops new leases; Resume restores eligibility;
Execute can restart a drained worker loop. A blocked task or conflict may require
a revised run; approving a command does not automatically replay an uncertain task.

Current GUI scope is macOS 14+. Windows/Linux retain the CLI and control API;
no cross-platform desktop GUI is claimed. Packaging/launch instructions are added
with the desktop distribution ticket. Paid-provider and hosted platform gates remain.
