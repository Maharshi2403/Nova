# Local packaging and release checklist

Nothing here publishes, pushes or tags. Current bundles are local candidates until
hosted Windows/macOS checks and an authorized budgeted live smoke are reviewed.

## Build, package, install smoke

```sh
uv sync --locked --project intelligence
uv run --locked --offline --project intelligence python scripts/prepare_release_cache.py
cargo test --locked --offline --workspace
cargo build --release --locked --offline --workspace
uv run --locked --offline --project intelligence python scripts/package_release.py
uv run --locked --offline --project intelligence python scripts/smoke_bundle.py --archive-dir dist
```

Initial dependency/build caches must be warm for offline operation. Pinned wheel
build constraints are in `scripts/build-constraints.txt`. Native Rust dependencies
use Cargo.lock. The Python runtime dependency export uses uv.lock with hashes.
The archive contains native CLI/daemon binaries, the pure-Python intelligence wheel,
locked runtime requirements, an offline demo, documentation, an unavailable model
example and build/checksum metadata. No token, database, user worktree or credentials
are included. Dependencies are not vendored: a fresh machine must populate its uv
cache from a package registry before an offline installation can succeed.
The optional `prepare_release_cache.py` step may download hash-verified dependencies
into a temporary venv/cache, then removes that venv. It makes no provider calls.
It also works inside an extracted bundle using its requirements.lock.txt.

The smoke script verifies every archived file and rejects unexpected paths, creates
a fresh venv, installs hash-pinned dependencies and the wheel from artifacts, then
executes a real three-task CLI workflow. It does not use the development editable
package. Native binaries are for the build machine's OS/architecture; CI packages
each native platform separately.

Archives have sorted entries, fixed timestamps/permissions and SHA256SUMS plus an
external archive checksum. Repeat packaging of identical binaries/source/wheel
inputs must match byte-for-byte. This does not claim cross-host native binary
identity or that code signing/notarization is complete. BUILD.json discloses the
source revision and tracked-dirty status. The initial bundle can be built before
the packaging commit; rebuild after committing to record the final source revision.

## Required sign-off before a public release

- [ ] Native Windows and macOS full CI, including 100 tasks, forced restart and cancellation.
- [ ] All completed tasks in fixtures have verification and task/integration SHAs.
- [ ] Fresh extracted-bundle installation succeeds; checksums and contents inspected.
- [ ] Credentials absent from artifacts/logs; threat model and limitations reviewed.
- [ ] Explicitly authorized live-provider smoke with account-specific model/cost cap;
      no automatic paid CI calls.
- [ ] Review migrations and take a private state backup before operator upgrades.
- [ ] Signing/notarization/distribution requirements decided per target platform.
- [ ] Only after explicit release authorization, consider a versioned tag such as
      v0.1.0. Tag creation, push and publication are separate manual decisions.

No hosted CI or paid smoke result may be inferred from a successful local build.

## Use an extracted bundle persistently

The smoke venv is disposable. For actual use, create a venv in the extracted bundle
with `uv venv runtime`, install `requirements.lock.txt` with `uv pip install
--require-hashes --python PYTHON -r requirements.lock.txt`, then install the wheel
with `uv pip install --no-deps --python PYTHON wheels/harness_ai-0.1.0-py3-none-any.whl`.
Replace PYTHON with `runtime/bin/python` on Unix or `runtime/Scripts/python.exe` on
Windows. Add `--offline` when caches are prepared. Run `bin/harnessd` with a dedicated
private `--data-dir`, and pass this venv's absolute Python path to `nova execute`.
The compiled binaries and Python wheel both report version 0.1.0.
