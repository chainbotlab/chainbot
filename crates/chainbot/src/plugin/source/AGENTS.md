# Local Rules

## Scope
- Position: Remote plugin source discovery and install subtree within the `plugin` subsystem.
- Logic: Owns source locator parsing, source manifests, transport materialization, install preparation, safe filesystem replacement, and rollback primitives for remote plugin packages.
- Constraints: Keep runtime plugin contract concerns in `../contract.rs` and runtime host execution in `../host.rs`; this subtree serves source discoverability and installation only.

## Members
- `mod.rs`: Internal source-install subsystem boundary.
- `locator.rs`: Source locator types for github/git targets and refs.
- `manifest.rs`: `chainbot-plugin-index.toml` and package-local `config.toml[source]` contracts plus CLI-facing source read models.
- `transport.rs`: Source transport materialization into temporary repo roots.
- `discover.rs`: Single/multi-plugin repo discovery, selection, and metadata projection.
- `prepare.rs`: Direct/build-required artifact preparation and validation.
- `install.rs`: Staging, backup, replace, and rollback transaction helpers.
- `fs.rs`: Path safety, symlink rejection, recursive copy, and temp filesystem helpers.
