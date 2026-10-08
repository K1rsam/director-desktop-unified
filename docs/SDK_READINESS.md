# Official SDK readiness without generation

The native `readiness.check` action for `codex_sdk` calls
`readiness_sdk::check(settings, app_resource_dir)`. Native action authorization is
separate from SDK generation. `textSdk.status` remains a static capability
description; it is not a successful connection test. Real SDK execution remains
disabled in the existing transport.

## What a successful check proves

- Exact official `@openai/codex-sdk@0.160.1` package metadata, JavaScript, and
  Apache-2.0 LICENSE match fixed SHA-256 identities
- Bridge, health entrypoint, lock, and notices match the compiled app
- An existing absolute native Node binary was found, hashed, and actually ran
  the genuine SDK import and `director-sdk-bridge/v1` hello/inspect/shutdown
- Node version is 18 or later; the worker exited and its disposable state was
  removed

It returns `provider-readiness/v1`: installation/protocol `ready`, authentication
and catalog `not_checked`, network `not_tested`, and generation
`{enabled:false,testPerformed:false,calls:0}`. It does not describe a model or an
account as connected. No Codex object is constructed, no runtime executable is
resolved or spawned, and no account/model/network request is sent.

## Native paths and isolation

Release builds load `<Tauri resource_dir>/sdk-readiness`. The resource directory
is passed by the native host, never read from IPC settings. Debug development
builds can fall back to the verified staged resources or sibling SDK source.
Portable Windows clients instead select the adjacent
`.client-resources/<actual EXE SHA256>/sdk-readiness` directory. The current and
previous EXEs each keep their own resources; a Cargo candidate under `release/deps`
can select the same hash directory in its parent release directory. Copying only
the EXE omits this resource set and cannot establish SDK readiness. The published
current/previous entry points default to `apps/desktop/.local/client`, separate
from Cargo's `src-tauri/target` artifacts. `ReleaseDirectory` can select another
publish directory outside the Cargo target tree.

`desktop:update` prepares a candidate by default; `-BuildOnly -CandidatePath`
stages an explicitly supplied prebuilt candidate without running the compiler.
The launchable `director-desktop-next-candidate.exe` is staged in the publish
directory beside its hash-bound resources; Cargo outputs remain build artifacts.
Only `-PublishOnly -CandidatePath` rotates the current/previous EXEs. The launcher
uses the same default publish directory and accepts `ReleaseDirectory` for an
explicit custom location. Publish the resources produced with that candidate,
retaining its hash directory when keeping the previous version.
After successful publication, the script removes only its own recorded,
byte-matching candidate from that client directory. An occupied candidate is
retained with a message and can be cleaned by repeating PublishOnly after it
closes; caller-supplied external candidate files are preserved.

Node is not included in either the portable resources or the Tauri bundle. SDK
readiness requires an installed native Node executable (18 or later), selected
by the validated installation locations or `textSdkNodePath`; a missing Node
does not authorize installation, runtime execution, or a CLI fallback.

`textSdkNodePath` may select an existing absolute native Node executable. Without
it, macOS checks Homebrew/system Node paths and Windows checks native Node install
locations. No shell/PATH wrapper, installer, download, login, or stored credential
directory is used. A missing Node reports `SDK_NODE_MISSING` with SDK-installed
status preserved. A selected invalid path is reported rather than falling back
to another executable.

Each check creates a private disposable directory. The worker gets `env_clear`
with HOME, USERPROFILE, CODEX_HOME, config/cache/temp paths pointing there. Windows
adds only its validated system-root location. Worker stdin is at most 8 KiB,
stdout at most 32 KiB, stderr at most 8 KiB, with an 8-second outer deadline and
bounded termination/reaping. The worker itself accepts only the three fixed,
ordered control frames. Fixture flags, caller-selected bridge/resource paths,
historical synthetic identities, execute/resume, and additional fields are
rejected. Raw stderr and account data are never returned in the readiness DTO.

## Packaging

Run `npm ci --ignore-scripts --omit=optional --no-audit --no-fund` inside
`sdk-bridge` before building. Tauri's before-dev and before-build hooks invoke
`sdk-bridge/scripts/package-readiness.mjs`; the helper performs no installation
or network access. Missing or mismatched official dependencies fail the build.

The helper stages exactly seven files plus an inventory manifest under
`apps/desktop/src-tauri/resources/sdk-readiness`, and Tauri maps that directory to
`sdk-readiness` in application resources. Staged files are ignored by version
control. The package includes the unmodified SDK JS and metadata, exact upstream
LICENSE, bridge and health source, notices, and the lock. It does not include
test fixtures, account directories, Node, or optional Codex platform binaries.
Only Node built-ins are imported by the official SDK during health; Codex runtime
resolution occurs in SDK construction, which this check never performs.

Native macOS and Windows packaging/signature/runtime availability still require
their own native acceptance; Linux checks do not establish those outcomes.

## Offline verification

- `npm run verify` in `sdk-bridge`: all bridge tests plus packaged health from an
  isolated relocated resource directory, with no source-runtime dependency
- `cargo test --offline --manifest-path apps/desktop/src-tauri/Cargo.toml
  readiness_sdk::tests`: genuine package/worker integration, resource relocation,
  tampering, missing Node, rejected fixture/settings/wrappers, framing and I/O
  bounds, cleanup, and conservative result claims
- Run the same Rust filter with `--release` to verify that relocated bundled
  resources work without the debug source fallback and that missing production
  resources still fail closed

Tests require actual installed Node and the exact official SDK; prerequisites
are never silently skipped. Test-only preloading instruments Node child-process,
network, HTTP, HTTP/2, and fetch entrypoints and fails on any attempted effect.
The native Rust integration also checks that no unrelated environment key was
inherited and that home/state directories remain empty after shutdown. This
preload is never used by production and cannot be requested over IPC.
