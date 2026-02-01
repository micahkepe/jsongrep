# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/micahkepe/jsongrep/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/micahkepe/jsongrep/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/micahkepe/jsongrep/releases/tag/v0.3.0
[0.2.0]: https://github.com/micahkepe/jsongrep/releases/tag/v0.2.0
[0.1.2]: https://github.com/micahkepe/jsongrep/releases/tag/v0.1.2
[0.1.1]: https://github.com/micahkepe/jsongrep/releases/tag/v0.1.1
