# Release metadata and dry-run validation

`release-manifest.json` is the source of truth for the version expected in each checked-in publication surface. Artifacts remain independently versioned: the Rust engine, Node SDK, Python SDK, parsers, adapters, and proxy do not have to share a number.

The manifest intentionally declares each artifact's publication destinations and every file that must agree with that artifact's version. For example, the Node SDK version is checked in its package manifest, lock file, MCP client identity, and MCP registry package entry. The Python SDK is checked in its project metadata, lock file, client identity, and registry entry.

Run the dry-run validator from the repository root:

```bash
cargo run -- release-validate
```

To retain the machine-readable result:

```bash
cargo run -- release-validate --output release-validation.json
```

The command performs no registry, Git, tag, or package mutations. It exits 2 if a source is missing, cannot be parsed, has no string version at the declared selector, or differs from the manifest. Source paths must be repository-relative, may not contain parent traversal, and may not resolve outside the repository through a symlink. It exits zero only when every declared source agrees.

## Updating a version

1. Decide which independently published artifact is changing.
2. Change its version once in `release-manifest.json`.
3. Update every source declared for that artifact; do not change unrelated artifacts merely to make the numbers uniform.
4. Run `cargo run -- release-validate`.
5. Run the artifact's tests and package/build dry run.
6. Review the generated package contents and changelog before any publish or tag operation.

Adding a new publishable package requires adding it to the release manifest in the same change. Private test packages and the documentation website are not publication artifacts and are intentionally absent.
