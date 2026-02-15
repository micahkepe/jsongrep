# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0] - 2026-02-15

### Added

- `-F` / `--fixed-string` CLI flag that treats the query as a literal field name
  and searches at any depth (equivalent to `(* | [*])*."<literal>"`)
- `--with-path` / `--no-path` flags for controlling path header display
- TTY-aware path header suppression: headers are shown when output is a
  terminal and hidden when piped, following ripgrep conventions

### Fixed

- Quoted field names with special characters (e.g., `/endpoint`) now correctly
  round-trip through parsing, escaping, and matching
- Don't display root path in colorized output - no longer prints `:` when
  no query is provided (e.g, `cat data.json | jg ""`)
- `jg generate man` now correctly prefixes all subcommand man pages with
  `jg-` (e.g., `jg-generate-shell.1` instead of `generate-shell.1`)
- `jg generate man` now overwrites existing man pages instead of failing
  with `AlreadyExists`, making version upgrades seamless

### Changed

- **BREAKING**: `jsongrep::utils::write_colored_result` now takes a
  `show_path: bool` parameter to control path header display
- Updated README usage examples to reflect `-F` flag and current output format
- Updated README with more examples and comparisons to `jq`

## [0.5.1] - 2026-02-14

### Fixed

- Updated README examples to reflect the new path-prefixed output format
- Updated library dependency version in README from `0.3` to `0.5`

## [0.5.0] - 2026-02-14

### Added

- Syntax-highlighted JSON output using the `colored` crate — keys in cyan,
  strings in green, numbers in yellow, Booleans in bold yellow, null in
  dimmed red
- Each query result now displays its matched JSON path as a colored header
  (e.g., `prizes.[4].laureates.[1]:`) above the value
- `Display` impl for `PathType` for human-readable path rendering
- New `utils` module with `write_colored_result` for colorized output and
  `depth()` (moved from `lib.rs`)

### Changed

- **BREAKING**: CLI output format changed from a single JSON array of all
  results to individual values, each preceded by its matched path. Scripts
  parsing the old `[...]` array output will need updating.
- **BREAKING**: `jsongrep::depth()` moved to `jsongrep::utils::depth()`
- `PathType` is now publicly re-exported from the `query` module
- Input parsing extracted into `parse_input_content` function
- Output uses a single locked `BufWriter<Stdout>` with explicit flush

### Fixed

- Broken pipe errors when piping to `less` or `head` are now silently
  handled instead of printing an error

## [0.4.1] - 2026-02-01

### Added

- `field!` macro for constructing field queries (e.g., `field!("foo")` =>
  `Query::Field("foo".to_string())`)
- `Query::field` method for constructing field queries from type `T: Into<String>`
  for convenience (e.g., `Query::field("foo")`)

### Fixed

- Fixed incorrect example query in README

## [0.4.0] - 2026-02-01

### Changed

- Removed `tokenizer` module from public API (was unused)

### Documentation

- Documented experimental regex support for pattern matching in queries
- Updated README with regular path syntax description

### Internal

- Addressed Clippy lints

## [0.3.0] - 2025-11-21

### Changed

- **BREAKING**: Migrated from custom `JSONValue` type to `serde_json::Value` for
  better compatibility with the Rust ecosystem
- **BREAKING**: Removed custom `schema` module (JSON schema validation
  functionality)
- Simplified JSON handling by leveraging `serde_json` directly
- Updated all query engine implementations to work with `serde_json::Value`

### Fixed

- Fixed JSON parsing in CLI to properly parse input files instead of wrapping
  them as strings

### Internal

- Refactored test utilities to use `serde_json::Map` instead of `HashMap`
- Moved `depth()` function from `JSONValue` method to standalone function in `lib.rs`
- Cleaned up type conversions throughout the codebase

## [0.2.0] - 2025-08-14

### Added

- Shell completion and man page generate with `generate` subcommand
- Pull request template for GitHub

### Changed

- Updated README with new instructions for `generate` subcommand and updated
  grammar syntax description

### Fixed

- Track `Cargo.lock` for dependencies
- Various Clippy warnings

## [0.1.2] - 2025-08-09

### Fixed

- Metadata in Cargo.toml had incorrect homepage URL
- GitHub Actions workflow to create GitHub releases

## [0.1.1] - 2025-08-09

### Added

- Initial release
- Support for simple queries and wildcards
  - Field access, index access, and wildcard access
  - Sequences and disjunctions
  - Kleene star
  - Optionals

[Unreleased]: https://github.com/micahkepe/jsongrep/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/micahkepe/jsongrep/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/micahkepe/jsongrep/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/micahkepe/jsongrep/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/micahkepe/jsongrep/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/micahkepe/jsongrep/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/micahkepe/jsongrep/releases/tag/v0.3.0
[0.2.0]: https://github.com/micahkepe/jsongrep/releases/tag/v0.2.0
[0.1.2]: https://github.com/micahkepe/jsongrep/releases/tag/v0.1.2
[0.1.1]: https://github.com/micahkepe/jsongrep/releases/tag/v0.1.1
