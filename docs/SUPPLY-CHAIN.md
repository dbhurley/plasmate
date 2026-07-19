# Supply-chain policy

Plasmate treats dependency metadata, lockfiles, CI actions, build tools, and
coverage evidence as executable inputs. Pull requests and pushes to `master`
run the same read-only dependency-security workflow; a weekly run refreshes
the advisory data even when the repository is idle.

## Trust boundaries

- Rust, npm, Python, and Go advisories come from their ecosystem databases.
  A clean scan means no advisory was known to that database at scan time; it
  is not a guarantee that a dependency is defect-free.
- Every remote GitHub Action is selected by a full 40-character commit SHA.
  The adjacent version comment is for humans and has no authority. Checkout
  credentials are not persisted. Workflow permissions default to
  `contents: read`; only the release and container-publish jobs receive the
  specific write permissions they require.
- The crate declares Rust 1.88 as its minimum. CI compiles both default and
  `plugins` configurations with exactly 1.88; this is the current whole-project
  floor even though Wasmtime 36 itself declares 1.86.
- npm security scans install lockfiles with lifecycle scripts disabled. Normal
  CI separately builds and tests packages from `npm ci`, including scripts the
  package itself owns.
- The hashed Python audit lock covers every external mandatory, build, and
  development dependency declared by the five first-party Python project
  roots. `scripts/check-python-audit-input.py` prevents metadata from silently
  escaping that input. The lock is resolved for CI's CPython 3.11/Linux target.
- The Browser Use integration does not import the `browser-use` package. Its
  unused convenience extra was removed after upstream pinned a vulnerable
  `pypdf`; consumers that independently install Browser Use own that separate
  dependency graph.
- Public-web coverage is untrusted, variable evidence. Scheduled HTML and JS
  runs have read-only repository permissions and upload 14-day artifacts.
  Updating a checked-in scorecard requires an explicit review and commit.

## Reproduce the audits

Run npm production and complete audits for every lock root:

```bash
for directory in packages/som-parser-node sdk/node smoke integrations/vercel-ai website; do
  npm ci --prefix "$directory"
  npm audit --prefix "$directory" --omit=dev
  npm audit --prefix "$directory"
done
```

Audit the other ecosystems with pinned scanner releases:

```bash
cargo install cargo-audit --version 0.22.2 --locked
cargo audit --file Cargo.lock

python3 scripts/check-python-audit-input.py
python3 -m pip install pip-audit==2.9.0 uv==0.11.28
pip-audit --requirement security/python-audit-requirements.lock --disable-pip
uv export --project sdk/python --locked --format requirements-txt \
  --no-emit-project --output-file /tmp/plasmate-sdk-python-audit.txt
pip-audit --requirement /tmp/plasmate-sdk-python-audit.txt --disable-pip

cd sdk/go
go install golang.org/x/vuln/cmd/govulncheck@v1.6.0
govulncheck ./...
```

## Verified baseline (2026-07-19)

| Dependency root | Production vulnerabilities | Complete/dev vulnerabilities |
|---|---:|---:|
| `packages/som-parser-node` | 0 | 0 |
| `sdk/node` | 0 | 0 |
| `smoke` | 0 | 0 |
| `integrations/vercel-ai` | 0 | 0 |
| `website` | 0 | 0 |

The same verification found zero known vulnerabilities in `Cargo.lock`, the
hashed multi-project Python lock, `sdk/python/uv.lock`, and the Go SDK. RustSec
also emitted the five non-vulnerability maintenance warnings detailed below.
The npm production column is `npm audit --omit=dev`; the complete column is an
unfiltered `npm audit`, so development tools are not hidden.

Regenerate the Python lock only after reviewing the resolver diff:

```bash
uv pip compile security/python-audit-requirements.in \
  --python-version 3.11 \
  --python-platform x86_64-unknown-linux-gnu \
  --generate-hashes \
  --output-file security/python-audit-requirements.lock
```

## Refresh immutable action pins

Dependabot proposes Action updates, but never auto-merges them. Verify the
release tag in the action's own repository and resolve it directly:

```bash
repository=actions/checkout
tag=v4.3.1
git ls-remote --tags "https://github.com/${repository}.git" \
  "refs/tags/${tag}" "refs/tags/${tag}^{}"
```

For an annotated tag, use the peeled `^{}` commit, not the tag-object SHA. For
a lightweight tag, use the single returned SHA. Replace the workflow ref,
retain the readable tag comment, inspect the upstream diff, and run:

```bash
python3 scripts/check-workflow-action-pins.py
```

Pinning limits tag-retargeting risk but does not make third-party action code
trusted. Release changes still require human review, and secrets are exposed
only to the job that needs them.

## Automated maintenance

Dependabot checks Cargo, GitHub Actions, Go, all five npm lock roots, all five
Python project roots, and the central Python audit lock weekly. Minor and
patch updates are grouped, concurrent pull requests are capped, major updates
remain isolated, and auto-merge is intentionally disabled.

Residual limitations are explicit: advisory databases can lag disclosure;
the Python lock is Linux-specific; Rust and Go scanners fetch current advisory
data during CI; and artifact scorecards are not promoted automatically. The
Rust audit currently reports five non-vulnerability warnings without an ignore
list: `adler` and `paste` are V8 build dependencies, `rustls-pemfile` is pending
migration to the replacement PEM API, and `rand` 0.8/0.9 are covered by an
unsoundness warning specific to a custom-logger interaction. Known
vulnerabilities, including the former Wasmtime sandbox-escape advisories, are
fixed and still fail the workflow.
