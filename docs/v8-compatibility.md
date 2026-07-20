# V8 compatibility policy

Plasmate deliberately pins `v8` to `139.0.0` (embedded V8 `13.9.205.15`). That
binding was released on 2025-07-24. It is the highest API-compatible release
verified on 2026-07-19, not the current upstream release and not a claim of
current V8 security support. The project MSRV is Rust 1.88 (the binding's own
recorded toolchain is 1.86 and its crate metadata uses edition 2024; Plasmate
uses edition 2021). The authoritative machine-readable target and build matrix
is [`compatibility/v8.json`](../compatibility/v8.json).

## Why this is staged

The observed upstream crate is `150.2.0`, released 2026-07-16 (embedded V8
`15.0.245.2`, Rust 1.91).
A disposable full-workspace build was attempted. It fails across runtime, DOM,
and module callbacks because recent rusty_v8 versions require pinned scopes
(`PinScope`/`PinnedRef`) where this code owns mutable `HandleScope` and
`CallbackScope` values. The incompatible line is present by 140.x. Treating that
as a dependency-only update would conceal a large memory-safety-sensitive API
migration.

This 11-major-version gap is a critical follow-up. The next upgrade must be an isolated runtime refactor: migrate all scope and
callback signatures together, retain the process supervisor boundary, and pass
the entire JS runtime and native ES-module suite before changing the pin. A
newer binding may ship only after it compiles on the project MSRV, the native
runtime/module and supervisor containment suites pass, and its embedded-V8
security delta is reviewed. Do not relax the exact version requirement to a
broad major range.

## Build and target assumptions

Default builds download a prebuilt static archive from rusty_v8's GitHub
releases. `V8_FROM_SOURCE=1` opts into a source build and requires Python 3,
curl, and a C++ toolchain; Linux also needs glib development headers, macOS needs
Xcode command-line tools, and Windows is 64-bit only. The supported archive
matrix is macOS x86_64/aarch64, Linux x86_64/aarch64, and Windows x86_64.

Every binary embeds `deno_core_icudata` `0.74.0`, the ICU 74 common-data
package matching the ABI exposed by `v8` `139.0.0`. V8 must receive that data
before platform initialization; starting without it can make ordinary
`Intl.DateTimeFormat` construction terminate the process with a misleading
fatal out-of-memory diagnostic. `PLASMATE_ICU_DATA` and `icudt74l.dat` or
`icudtl.dat` beside the executable remain supported as operator overrides. ICU
searches packages in registration order: a valid external package is registered
first and the embedded package second, giving the override precedence with a
complete fallback for missing resources. Missing, unreadable, or rejected
external packages are skipped and their temporary allocation is reclaimed.
Keep both exact dependency pins and the `Intl.DateTimeFormat` regression test
in lockstep during the next V8/ICU upgrade.

Run the deterministic drift check whenever Cargo or the V8 pin changes:

```bash
python3 scripts/check-v8-compatibility.py
```

The recorded upstream sources are the [139.0.0 crate](https://docs.rs/crate/v8/139.0.0),
the [150.2.0 crate](https://docs.rs/crate/v8/150.2.0), the
[rusty_v8 repository](https://github.com/denoland/rusty_v8), and the
[official V8 repository](https://chromium.googlesource.com/v8/v8.git/).
