# tooling

## scope

protocol CLI, Rust SDK, and canonical metadata schemas. see [tooling documentation](https://fossa.ag/docs/providers/cli).

- `cli/`: noninteractive Rust CLI using the SDK.
- `sdk/rust/`: protocol SDK with offline builders, typed accounts, pooled RPC, and signing.
- `schema/`: canonical metadata schema, validator, and conformance cases.

## workflow

- dependency updates and security fixes are manual; retain vulnerability alerts and dependency inventories.
- read the relevant implementation before editing. clarify uncertain requirements and keep changes focused.
- deployments and package publication require an explicit request. tooling releases do not authorize onboarding, signing, program deployment, or fund movement.
- preserve unrelated configuration, signer files, metadata commitments, and transaction journals.

## implementation

- CLI commands inspect or plan by default; execution requires an explicit flag. never prompt interactively. return machine-readable output and useful errors and help.
- keep SDK transport pooled and reusable. preserve exact accounting, account bindings, and instruction compatibility. custom RPC and metadata hosting remain supported.
- preserve canonical metadata validation and exact document bytes. consumers vendor independent copies without sibling runtime dependencies or schema downloads.
- synchronize explicit targets with `bun schema/sync.ts --check` or `--write`: use repeatable `--rust PATH`, `--frontend PATH`, or `--indexer PATH` options. inspect changes before writing consumer copies.
- CLI help and linked documentation describe setup and usage. label fenced code blocks with their language.
- keep readmes to documentation links: one properly capitalized link per component, each on its own line. public documentation explains integration contracts without private operational details.

## configuration and security

- configuration and signer files stay outside git; examples contain placeholders only.
- never log or commit secrets, private user state, or signed transaction bytes.
- preserve transaction journals and reconcile uncertain submissions before preparing replacements.

## verification

- before a requested commit or push, run `cargo fmt --all --check`, `cargo test --workspace --locked`, and `cargo clippy --workspace --all-targets --locked -- -D warnings`, then review the final diff.
- tests use synthetic keys and mock RPC only. schema changes also require conformance coverage and checks against affected vendored copies.

## commits and releases

- create commits and push only when explicitly requested.
- use `[type] short description`; allowed types are `[fix]`, `[feat]`, `[add]`, and `[update]`.
- messages must be one lowercase line. combine multiple changes with `+`.
- never add attribution or `co-authored-by` trailers.
- bump affected versions with semver for behavior or release changes. tag requested releases `vX.Y.Z`.
