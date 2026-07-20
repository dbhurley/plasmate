# Publishing Plasmate to the MCP Registry

The [MCP Registry](https://registry.modelcontextprotocol.io/) entry is an
installable server declaration, not only a discovery record. The checked-in
metadata declares the native engine as a version-pinned OCI image:

```text
ghcr.io/plasmate-labs/plasmate:v0.6.0
```

Registry clients run the image over stdio and pass the positional `mcp`
argument. With the Docker `ENTRYPOINT ["plasmate"]`, that starts `plasmate mcp`.
The npm and PyPI packages named `plasmate` are client SDKs and are deliberately
not registry runtime packages.

This is the next v0.6.0 release candidate, not a currently available image. The
v0.5.1 image was built before the MCP ownership label existed and must remain
immutable; it cannot be relabeled or republished as the corrected Registry
runtime. Do not publish this Registry entry until the v0.6.0 release workflow
builds the newly labeled image and an anonymous pull succeeds.

## Release invariants

Before publishing, all of these values must describe the same release:

- `release-manifest.json` declares the Rust artifact version and both `ghcr`
  and `mcp_registry` publication destinations.
- `server.json` uses `registryType: "oci"`, package version `0.6.0`, identifier
  `ghcr.io/plasmate-labs/plasmate:v0.6.0`, stdio transport, and a required
  positional package argument whose value is `mcp`.
- `Dockerfile` retains `ENTRYPOINT ["plasmate"]` and the label
  `io.modelcontextprotocol.server.name="io.github.plasmate-labs/plasmate"`.
- The immutable versioned GHCR image is publicly pullable without repository
  credentials before the Registry entry is published. Do not substitute
  `latest`; Registry installs must be repeatable.

The checked-in release validator enforces the local parts of this contract:

```bash
cargo run --locked -- release-validate
```

## Install and validate the publisher

Download `mcp-publisher` from the official Registry release page into a trusted
location, then verify it is available:

```bash
mcp-publisher --help
mcp-publisher validate
```

`mcp-publisher validate` validates `server.json` against the official schema.
It does not prove that the declared image is already present or public in GHCR,
so verify an anonymous pull separately after the release workflow publishes the
versioned image. A result that succeeds only after `docker login` is not enough
for Registry consumers:

```bash
docker manifest inspect ghcr.io/plasmate-labs/plasmate:v0.6.0
```

## Authenticate and publish

The server name is `io.github.plasmate-labs/plasmate`; authentication therefore
requires authority for the `plasmate-labs` GitHub organization.

```bash
mcp-publisher login github
mcp-publisher publish
```

Publishing is an explicit post-image release operation. Do not publish from an
untagged candidate, before an anonymous GHCR pull succeeds, or with a moving
image tag.

## Verify the registry record

```bash
curl "https://registry.modelcontextprotocol.io/v0.1/servers?search=io.github.plasmate-labs/plasmate"
```

Confirm that the returned version, OCI identifier, stdio transport, and `mcp`
package argument exactly match `server.json`. A valid local schema result does
not replace this post-publication check.

## Subsequent version bumps

For every Rust engine release:

1. Update the Rust artifact version and all declared sources through
   `release-manifest.json`.
2. Update both `server.json.version` and `packages[0].version`.
3. Update the OCI identifier tag to exactly `v<new version>`.
4. Run `cargo run --locked -- release-validate`, the official schema validator,
   and the normal release test suite before creating the immutable Git tag.
5. Wait for the matching GHCR image to publish and become anonymously pullable.
6. Run `mcp-publisher publish`, then inspect the public Registry record.

Never point the Registry entry at npm or PyPI unless that package independently
installs and launches the native MCP server; SDK importability is not a server
runtime contract.
