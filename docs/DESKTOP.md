# Native desktop dashboard

Nova's macOS desktop client is a native SwiftUI island panel. It uses a bounded JSON stdin bridge
to the Nova CLI; the local daemon requires no bearer token or sign-in. Provider
API keys remain private and do not appear in arguments or URLs. The daemon still rejects browser origins.
There is no hosted dashboard or telemetry. Live models receive approved task context.

The panel opens as a 210 × 44 point dark status pill below the top-center notch/island
area. Expand it to 360 × 340 points for Work, Projects and Models. Its menu-bar
Nova logo restores a hidden panel; repeated launches reuse the same panel for
that state directory. It floats without replacing the active app. Setup and task
forms are compact on-demand sheets. The editable logo is `apps/desktop/NovaLogo.svg`.

The panel shows projects, paged runs, task states, plan versions, model usage,
scoped command approvals, verifier results and task/integration commit identities.
Only durable verified/integrated state is presented as completed. Unknown costs
remain unknown. The activity view shows up to the first 1,000 events, with the last
30 of that page visible in Details. CLI event cursors remain available for larger histories.

Provider setup uses a hidden key field, account-model discovery and macOS Keychain.
No generation request is made when connecting a provider. Unknown model metadata
still needs `nova models configure`; API keys alone cannot establish unknown prices
or structured-output support. Routes can be pinned or returned to complexity/risk
defaults in the Model routes view.

Add an existing committed Git repository with a name and explicit local trust choice.
A project can be added while it has uncommitted changes; import leaves those files
untouched. Execution review defaults to **Use committed snapshot**: it uses the last commit
in isolated task/integration worktrees and leaves all unfinished files untouched.
Uncommitted changes are excluded from that source. Turn snapshot mode off only
when you want the strict clean-checkout workflow. The source revision is pinned
per run and reused on resume; Details shows its hash. Untrusted projects show “Trust needed” with an explicit Enable action. Enabling
local execution is a separate trust decision, and does not approve or execute a
plan. Empty project registrations can be removed without deleting repository files;
projects with runs are retained. Tests run with your user
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

## Build, install and open

```sh
cargo build --release --locked --offline --workspace
intelligence/.venv/bin/python scripts/build_desktop.py
intelligence/.venv/bin/python scripts/install_desktop.py
nova ui
```

The build needs the macOS SDK, installed base Python >=3.12, and a warm uv cache.
It creates `dist/Nova.app` and a desktop ZIP/checksum with a locked intelligence
venv and CLI inside. This is a local unsigned candidate, not a notarized public
release. Its venv depends on the configured installed base Python/stdlib; the ZIP
is not a standalone Python distribution for an arbitrary other Mac.

The installer copies Nova.app into `~/Applications`, updates `~/.local/bin/nova`,
and retains previous app/CLI versions. It does not change providers/projects or
start workers. Open Nova from Applications or `nova ui`; `nova ui --print-path`
shows the selected bundle without launching. A custom state folder is supported
with `nova --daemon-dir PATH ui`. Direct app launch uses Nova's normal user state.
The window is independent of the service, and the bundled CLI locates its own
intelligence runtime. Existing explicitly saved Python paths remain respected.

The task editor no longer pre-fills pytest for every repository. For documentation,
enter the allowed output folder and choose **Use documentation check**. The bundled
stdlib-only checker requires Markdown under those paths, at least 300 words, three
sections, a fenced Mermaid diagram, architecture/system-design text, and future
improvements or a roadmap. It reads only files inside the task worktree and has
file/byte limits. These are basic artifact checks, not a guarantee of prose quality.
Other tasks need an appropriate explicit test argv; no test framework is assumed.
