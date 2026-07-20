# Release metadata and dry-run validation

`release-manifest.json` is the source of truth for the version expected in each checked-in publication surface. Artifacts remain independently versioned: the Rust engine, Node SDK, Python SDK, parsers, adapters, and proxy do not have to share a number.

The manifest intentionally declares each artifact's publication destinations and every file that must agree with that artifact's version. For example, the Node SDK version is checked in its package manifest, lock file, MCP client identity, and MCP registry package entry. The Python SDK is checked in its project metadata, lock file, client identity, and registry entry.

Run the dry-run validator from the repository root with the committed lockfile:

```bash
cargo run --locked -- release-validate
```

To retain the machine-readable result:

```bash
cargo run --locked -- release-validate --output release-validation.json
```

The command performs no registry, Git, tag, or package mutations. It exits 2 if a source is missing, cannot be parsed, has no string version at the declared selector, or differs from the manifest. Source paths must be repository-relative, may not contain parent traversal, and may not resolve outside the repository through a symlink. It exits zero only when every declared source agrees.

## Updating a version

1. Decide which independently published artifact is changing.
2. Change its version once in `release-manifest.json`.
3. Update every source declared for that artifact; do not change unrelated artifacts merely to make the numbers uniform.
4. Run `cargo run --locked -- release-validate`.
5. Run the artifact's tests and package/build dry run.
6. Review the generated package contents and changelog before any publish or tag operation.

Adding a new publishable package requires adding it to the release manifest in the same change. Private test packages and the documentation website are not publication artifacts and are intentionally absent.

## Production release authorization

`.github/workflows/release.yml` has no manual publication path. It runs only
when a `v*` tag is pushed, and all builds wait for a fail-closed preflight. The
preflight verifies that:

- the event is a tag push;
- the Rust version is a stable `MAJOR.MINOR.PATCH` without prerelease or build
  metadata, and the tag is exactly `v` plus that version;
- the tag, event SHA, and checked-out commit are identical;
- the release commit is contained in the canonical repository's `master`;
- the latest matching check run for every required context completed
  successfully on the same SHA under the GitHub Actions app (`15368`).

Authorization runs before installing Rust or executing any candidate binary.
Only after it passes does the job run `cargo run --locked -- release-validate`
and `cargo publish --locked --dry-run`.

The required check names are deliberately literal because GitHub branch
protection and the Checks API use job names, not workflow step names:

```text
Minimum Rust 1.88
test (ubuntu-latest)
test (macos-latest)
action-manifest
Workflow trust policy
Rust advisory audit
npm audit (packages/som-parser-node)
npm audit (sdk/node)
npm audit (smoke)
npm audit (integrations/vercel-ai)
npm audit (website)
Python dependency audit
Go vulnerability audit
```

The crates.io, GHCR, and GitHub release jobs all depend on the preflight and
build matrix, use the `release` environment, and receive only their job-specific
permissions. Publication is serialized: the crate must publish first, the
container publishes second, and the public GitHub release is the final
coordination point. Because these are three separate jobs targeting the
protected `release` environment, GitHub normally creates three distinct
deployments and approval gates, not one approval for the entire chain. The
sequence is not transactional: if crates.io or GHCR succeeds and a later
approval or job fails, the release is partially published. Do not restart it as
a fresh release or move the tag; continue or recover the failed stage as a
release incident using the original immutable tag and workflow evidence.

Cargo resolution is locked in metadata validation, package dry-run, native and
cross builds, and publication. Preflight, artifact builds, and crates.io
publication all use exact Rust `1.88.0`, matching `package.rust-version` and the
required minimum-Rust check; the production workflow never resolves a floating
`stable` compiler. A failed or missing check, a newer failed attempt with the
same name, an action from another app, or a check for another SHA blocks the
release.

Run the deterministic authorization tests locally with:

```bash
python3 -m unittest scripts/test_verify_release_preflight.py -v
python3 scripts/check-workflow-action-pins.py
```

These tests do not call GitHub or publish anything. There is intentionally no
workflow-based nightly or prerelease escape hatch; use local locked builds for
dry runs.

## External repository prerequisites

The workflow cannot configure its own trust boundary. More importantly, GitHub
runs the workflow file from the tagged commit. Tagging an old commit can execute
that commit's older, unsafe `release.yml` without this preflight or environment.
Repository administrators must therefore complete these controls before any
new production tag is pushed:

1. Protect `master`, enforce protection for administrators, require a pull
   request, require branches to be current, block force-pushes and deletion,
   require conversation resolution, and require all 13 contexts above from app
   `15368`.
2. Configure a repository tag ruleset for `refs/tags/v*` that prevents update,
   deletion, and non-fast-forward changes and restricts tag creation to the
   release maintainers. Do not grant an Actions bypass; the workflow consumes a
   tag but never creates or moves one. Treat an administrator creation bypass
   as production access: the operator must verify that the target commit
   contains the hardened workflow before creating the tag.
3. Configure the `release` environment with required reviewers, prevent
   self-review when an independent reviewer exists, restrict deployments to
   protected `v*` tags, and store `CARGO_REGISTRY_TOKEN` only as an environment
   secret. Remove any repository- or organization-scoped copy of that token so
   an old workflow that does not declare the environment cannot read it. A sole
   maintainer can remain operable by selecting that maintainer as the required
   reviewer and leaving "Prevent self-review" disabled; this is an explicit
   confirmation gate, not independent approval. Add a second trusted reviewer
   and enable self-review prevention when independent authorization is required.
4. Run one exact-SHA release session. Require successful CI and dependency
   security on the candidate, then dispatch the isolated JS coverage workflow
   twice on that same SHA. Both newest scorecard checks must succeed within the
   24 hours before tag preflight. Download and validate both artifacts; retain
   their run IDs, attempts, logs, candidate SHA, corpus digest, and repository
   protection snapshots. Any newer failed scorecard blocks release.

The exact repository-side procedure is in `docs/P0-EXECUTION-PLAN.md`. It
dispatches all evidence-producing workflows on one candidate and validates two
independent `coverage_js` check runs. Production tag preflight queries GitHub's
exact-SHA check runs directly, rejects stale or newer failed JS evidence, and
requires the same 13 protected CI/security contexts used by `master`.

Workflow YAML does not prove that branch, tag, or environment protections are
active. Retain the read-only GitHub settings snapshots alongside the preflight
result and linked Actions evidence. If `master` moves before the tag is pushed,
rerun the release session on the new SHA; never combine evidence from commits.

Only after these controls and the exact-SHA release session are complete should
the maintainer create and push an annotated release tag. The production
workflow intentionally rejects prerelease and build-metadata versions because
it always publishes a stable GitHub release and advances the container's
`latest` tag. Because a protected release tag is immutable, run the local
checks before creating it:

```bash
git fetch upstream master
candidate=$(git rev-parse upstream/master)
test "$(git rev-parse HEAD)" = "$candidate"
cargo run --locked -- release-validate
cargo publish --locked --dry-run
cargo test --locked --workspace
cargo test --locked --workspace --all-features
git tag -s "v$(sed -n '/^\[package\]/,/^\[/s/^version = "\([^"]*\)"/\1/p' Cargo.toml)" "$candidate"
```

Review the tag and candidate SHA before pushing. A rejected preflight leaves the
tag in the repository but publishes nothing; correcting a bad immutable tag
requires an explicit administrator ruleset bypass and should be treated as a
release incident, not a routine retry.
