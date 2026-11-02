# Release installation tests

Test the candidate through the real mise and Homebrew executables before
publishing. Keep `v0.1.0` absent from production GitHub releases and tags until
release approval. Pushing a task branch must not start release jobs.

## Test matrix

| Host         | Manager             | Cases                                           |
| ------------ | ------------------- | ----------------------------------------------- |
| macOS arm64  | mise GitHub backend | Pinned, latest, reinstall, corrupted download   |
| Linux x86-64 | mise GitHub backend | Pinned, latest, reinstall, corrupted download   |
| macOS arm64  | Homebrew            | Fresh install, reinstall, upgrade, formula test |
| Linux x86-64 | Homebrew            | Fresh install, reinstall, upgrade, formula test |

Use disposable hosts with their normal Homebrew prefixes. Do not replace a
personal installation or assume Homebrew supports an arbitrary relocated
prefix. Keep mise data, cache, config, and state directories separate per case.
Run installs outside the source checkout with no existing `vpdf` on `PATH`.

## Candidate artifacts

1. Pin the reviewed Vpdf and tap commit IDs. Clone the Vpdf commit into an
   isolated local repository with independent refs. Add `v0.1.0` only there;
   never push it. Do not add a release tag to the shared task repository.
2. Run the existing CI recipe on each supported build host. Check that the
   isolated tag, Cargo version, and changelog agree. Linux builds run in a
   local container; copy only binaries and public PDFs to a remote test host.
3. Record the source revision, archive hashes, executable hashes, tool versions,
   and expected complete version text. Verify archive contents and executable
   permissions. Use these exact archives in both package-manager tests.
4. Obtain a public PDF with `mise run corpus`; use the corpus verifier and
   record its hash. Test `VPDF_PREVIEW=text vpdf PDF` against it and compare
   installed output with output from the candidate executable. Do not substitute
   a shell stub.

A local tag is a build fixture, not a published release. Preserve that
distinction in reports and never call a branch build a stable release.

## Mise without publication

Use the real [GitHub backend][mise-github] against a loopback release API that
serves the real candidate archives. Configure only the test tool's `api_url`;
keep the repository name, asset names, and normal asset autodetection unchanged.
Disable the versions host for the test and unset GitHub tokens. Reject fixture
requests outside the loopback host.

The fixture must implement release listing, latest, tag lookup, and asset
lookup/download routes used by the pinned mise version. Include release IDs,
tags, draft/prerelease status, asset names, sizes, digests, download URLs, and
checksum sidecars. Log requests so failures identify discovery versus download
or extraction. Verify custom API transport support first; if HTTPS is needed,
use a disposable trusted test certificate rather than disabling TLS validation.

1. With empty mise directories, resolve the tool at `0.1.0`, install it, locate
   its executable, and check its SHA-256 against the unpacked candidate.
2. Verify `--version`, `-V`, embedded notes, project URL, and headless
   text-preview output.
3. Reset state. Resolve `latest` with older stable, newer prerelease, and draft
   records present. Assert selection of the expected stable version.
4. Force reinstall and repeat identity and PDF checks. Confirm no user/global
   mise configuration changed.
5. Reset cache, corrupt the served archive while preserving its advertised
   checksum, and require installation failure. Also test a missing platform
   asset and an interrupted download; preserve any prior working install.

A direct HTTP-backend install or explicit asset URL can help diagnose
extraction, but cannot replace testing GitHub asset discovery.

## Homebrew without publication

Generate the candidate formula using the real tap updater in an isolated tap
checkout. A test adapter may replace its GitHub metadata lookup and download
URLs with loopback equivalents; use the real archive bytes and checksums.
Keep production updater validation enabled. Record the URL-only differences
between the test formula and the intended production formula.

1. Run `brew style`, then tap the isolated checkout under a test-only tap name.
   Install its `vpdf` in the disposable host's clean prefix.
2. Run `brew test` and verify the installed executable hash, version aliases,
   full notes, and real-PDF text-preview output. Reinstall and repeat the
   checks.
3. On a fresh host, install the currently pinned snapshot from its existing
   public release. Record its version and hash. Advance only the isolated tap
   to the candidate formula, run `brew upgrade`, and verify candidate identity.
   The old snapshot is an upgrade fixture; do not publish another snapshot.
4. Corrupt a candidate download with the original checksum retained and empty
   download cache. Require a checksum failure and preserve the working install.
5. Run `brew audit --strict` and record policy failures separately from runtime
   failures. Local fixture URLs may need a separate production-formula audit;
   never claim a loopback URL audit proves public download availability.

Homebrew's [formula guide][brew-guide] and [manual][brew-manual] define install,
test, reinstall, and upgrade behavior. Do not change the production tap or
remove its existing snapshot pins during this rehearsal.

## Publication boundary

These tests prove package-manager discovery against controlled metadata,
downloads, extraction, version output, PDF execution, and upgrade behavior.
They do not prove GitHub's public release listing, anonymous asset URLs,
production token permissions, deploy-key access, or real latest selection.
Draft GitHub releases are not a replacement: mise excludes drafts.

After explicit release approval, publish the reviewed stable tag and assets.
Repeat fresh pinned and latest mise installs, Homebrew installation and upgrade,
checksum and executable identity checks, and real-PDF checks against production.
Only then remove existing development releases and tags. Until approval, no
release, tag, asset, or live formula may be created, changed, or deleted by
tests.

## Evidence and acceptance

Record one result per matrix row: commits, OS/architecture, manager version,
commands, isolation paths, archive/executable/PDF hashes, selected release,
request trace, exit status, and differences from production. Keep logs and
artifacts outside Git and link a small report from the release handoff.

All positive cases must pass, negative cases must reject the altered inputs,
and installed bytes must match the candidate. Report a skipped host or any
untested production behavior as a gap. This document is a plan; it does not
record completed end-to-end tests.

[mise-github]: https://mise.jdx.dev/dev-tools/backends/github.html
[brew-guide]: https://docs.brew.sh/Formula-Cookbook
[brew-manual]: https://docs.brew.sh/Manpage
