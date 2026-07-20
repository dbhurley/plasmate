# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub Security Advisories for
`plasmate-labs/plasmate`. Do not include cookies, authorization headers, page
content, or other secrets in a public issue.

## Trust boundaries

Plasmate processes attacker-controlled URLs, HTML, JavaScript, redirects, DNS
answers, response bodies, and MCP/AWP/CDP inputs. These inputs are untrusted.
Stored auth profiles and the local auth bridge carry credentials and are a
separate, sensitive boundary. A local Plasmate process runs with the invoking
user's privileges; it is not a sandbox for malicious native plugins.

The default outbound policy permits only `http` and `https` destinations that
resolve exclusively to globally routable IP addresses. Loopback, private,
link-local, multicast, unspecified, reserved/documentation ranges, IPv4-mapped
IPv6 addresses, and known cloud-metadata hosts are rejected. Every redirect is
resolved and validated before it is followed. The reqwest DNS resolver repeats
the address check at connection time to defend against DNS rebinding.

Response `Content-Length` is checked against `PLASMATE_MAX_COMPRESSED_BYTES`
(default 8 MiB), and decoded body chunks are stopped at
`PLASMATE_MAX_BODY_BYTES` (default 16 MiB). Redirects default to five and can be
reduced with `PLASMATE_MAX_REDIRECTS`. External scripts and page JavaScript have
additional, smaller limits.

An explicitly configured HTTP/SOCKS proxy is a trusted network boundary. A
proxy can perform remote DNS resolution outside Plasmate's connection-time DNS
resolver. Plasmate still validates the destination before sending the request,
but operators must configure the proxy to block private and metadata networks.

AWP and CDP are unauthenticated control protocols and therefore refuse
non-loopback bind hosts. The daemon, auth bridge, and MCP Streamable HTTP server
also bind to loopback only.
Screenshot commands render policy-fetched HTML with Chrome networking forced
through a closed local proxy. Direct Chrome URL navigation cannot safely inspect
browser-managed redirects, so it is disabled unless the unsafe development
override is active. Chrome's process sandbox remains enabled.

## Auth bridge

Every auth-bridge endpoint requires `Authorization: Bearer <capability-token>`.
The bridge creates a 256-bit token at startup unless
`PLASMATE_AUTH_BRIDGE_TOKEN` supplies one of at least 32 characters. Treat this
token like a password and never place it in a URL. Auto-generated tokens are
written only to stderr for interactive bootstrapping, live for that bridge
process, and must not be copied into retained or shared logs.

Browser access is disabled unless `PLASMATE_AUTH_BRIDGE_ORIGIN` is the exact
`chrome-extension://<extension-id>` origin. Origins are checked on requests and
CORS responses; wildcard CORS is never enabled. Restarting without a configured
token rotates the capability.

Auth profiles and their master key are serialized through a process-shared
lock. New values are written to owner-only same-directory temporary files,
synced, atomically renamed, and followed by a directory sync. On Unix,
`~/.plasmate` and its profile directory are enforced as mode `0700`; the key,
profiles, temporary files, and lock are enforced as mode `0600`, and sensitive
reads use no-follow opens. If encrypted profiles exist but `master.key` is
missing, Plasmate fails closed instead of generating a key that cannot decrypt
them. Plaintext, legacy encrypted, and envelope-v1 profiles remain readable and
are migrated under the same lock.

## MCP Streamable HTTP

The HTTP MCP endpoint always requires a bearer capability token of at least 32
visible ASCII bytes without whitespace, compared with the same constant-time
helper used by the auth bridge. Set
`PLASMATE_MCP_HTTP_TOKEN` or pass `--token`; otherwise a fresh 256-bit token is
printed to stderr at startup. The server refuses non-loopback binds even when a
token is present, limits request bodies to 1 MiB, and applies a 60-second
handler deadline plus a 65-second whole-request deadline. Protocol sessions are
capped at 128 and expire after 30 minutes idle.

Requests without an `Origin` header are accepted for native clients. Browser
requests are forbidden unless their exact HTTP(S) origin is configured with
`--allow-origin`; no wildcard Origin is supported. The server does not emit CORS
response headers or support preflight, so direct browser-client compatibility
is not claimed. Capability tokens sent over plain HTTP must never leave the
host. Plasmate does not yet provide TLS or OAuth for a remotely exposed MCP
endpoint.

## Unsafe development override

Local integration fixtures must opt in explicitly. Setting
`PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK=1` disables private-network destination
protection for that process. It is intentionally verbose, unsafe, and must not
be set in services, shells used for normal browsing, CI against untrusted URLs,
or production. Unit tests use a test-only policy constructor instead of
weakening production defaults.

## Residual risks

Ordinary untrusted page JavaScript executes in a supervised child process with
bounded input/output, a hard deadline, an empty inherited environment, and
process-tree cleanup. This containment prevents fatal V8 failures from ending
the CLI, daemon, MCP, AWP, or CDP coordinator, but it is not a complete OS
sandbox. Direct `JsRuntime` construction and the explicit
`PipelineConfig::isolate_js = false` escape hatch remain in-process surfaces for
trusted tests or an already-supervised worker. Run Plasmate with least
privilege, do not expose control protocols to other users, and use an OS sandbox
for adversarial workloads. Auth-profile encryption protects files at rest; it
does not protect against another process already running as the same OS user.
