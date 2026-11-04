$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo fmt --all --check; if ($LASTEXITCODE) { exit $LASTEXITCODE }
    cargo clippy --workspace --all-targets -- -D warnings; if ($LASTEXITCODE) { exit $LASTEXITCODE }
    Push-Location intelligence
    try {
        uv run --locked ruff check .; if ($LASTEXITCODE) { exit $LASTEXITCODE }
        uv run --locked ruff format --check .; if ($LASTEXITCODE) { exit $LASTEXITCODE }
        uv run --locked mypy; if ($LASTEXITCODE) { exit $LASTEXITCODE }
    } finally { Pop-Location }
} finally { Pop-Location }
