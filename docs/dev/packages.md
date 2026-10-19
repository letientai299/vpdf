# Package ownership

Packages separate native document services, terminal resources, interface
behavior, and application effects. Keep feature state and transitions together;
split internal modules before adding another package.

## Owners and dependencies

The [dependency policy][policy] defines permitted internal edges. It applies to
normal, development, build, optional, and platform-specific dependencies.
External dependencies must preserve each owner's boundary.

| Owner                      | Responsibility                               |
| -------------------------- | -------------------------------------------- |
| [Model][model]             | Shared values, geometry, text reconstruction |
| [Search][search]           | Indexing, matching, index encoding           |
| [Reader][reader]           | Motions, selections, registers, jump history |
| [Protocol][protocol]       | Terminal input byte decoding                 |
| [PDF][pdf]                 | Native access, document jobs, content caches |
| [Terminal][terminal]       | Session, capabilities, graphics, spool files |
| [UI][ui]                   | Bindings, components, layout, theme, drawing |
| [Application][application] | Composition, persistence, CLI, OS effects    |

Model and protocol have no internal dependencies. Search consumes model;
reader and PDF consume model and search. Terminal consumes model and protocol.
UI consumes model, reader, search, and terminal. The application connects these
owners, consuming normalized input through terminal. Import owners directly;
do not add application forwarding exports.

MuPDF belongs only to PDF and the application. UI and terminal tests and
benchmarks must remain native-free, as must model, search, reader, and protocol.
Do not introduce a native fixture helper as a development dependency there.

## Cross-package contracts

Keep shared geometry and document values in model. PDF converts geometry to
native types; UI owns viewport layout, hit geometry, and prefetch decisions.
UI components own private state and expose transitions. The application adapts
native documents to reader and UI data-access traits, integrates job results,
and performs filesystem, process, clipboard, and URL effects.

PDF owns native handles, cancellation, document-cache provenance, and
[job resource configuration][jobs]. A render completion sink lets the
application connect terminal encoding without a PDF-to-terminal dependency.
Encoding stays on the worker before result delivery and waking. Preserve queue
bounds and spool lifetime when changing that contract.

Terminal owns negotiation and cleanup. Headless presenters take explicit
transport settings and an output sink; they must not open a terminal or probe
process-global capabilities. User reading state remains in the application,
separate from disposable PDF content caches. See [runtime ownership][arch] for
thread and lifecycle constraints.

## Focused checks and benchmarks

Select the owning Cargo package from the [workspace manifest][workspace]:

```sh
mise exec -- cargo nextest run --locked -p vpdf-ui
mise run bench:micro -- --package vpdf-ui
```

The same selectors work for each package, including `vpdf` for application
integration tests and benchmarks. [Development][development] lists focused
mise test tasks and real-PDF fixture preparation. Pure package benchmarks use
prepared, hash-verified data; PDF benchmarks open documents directly. Keep data
preparation outside measured loops. A component timing does not establish
whole-reader performance; retain integrated latency and live-terminal checks.

```sh
mise run check:packages
mise run test:packages
```

The verifier reads [Cargo metadata][metadata] with all workspace features and
without a platform filter. It checks declarations, package identities, and
transitive dependencies. Missing members or incomplete resolution fail the
check. `check`, `lint`, and `test` include this gate; CI uses those same tasks.
A dependency graph proves isolation of dependencies, not correct state
ownership or runtime behavior. Review those contracts and run the relevant
corpus, OCR, and terminal checks when they change.

[policy]: ../../scripts/check_packages.py
[workspace]: ../../Cargo.toml
[model]: ../../crates/vpdf-model/src/lib.rs
[search]: ../../crates/vpdf-search/src/lib.rs
[reader]: ../../crates/vpdf-reader/src/lib.rs
[protocol]: ../../crates/vpdf-protocol/src/lib.rs
[pdf]: ../../crates/vpdf-pdf/src/lib.rs
[terminal]: ../../crates/terminal/src/lib.rs
[ui]: ../../crates/ui/src/lib.rs
[application]: ../../src/lib.rs
[jobs]: ../../crates/vpdf-pdf/src/jobs/mod.rs
[arch]: arch.md
[development]: dev.md
[metadata]: https://doc.rust-lang.org/cargo/commands/cargo-metadata.html
