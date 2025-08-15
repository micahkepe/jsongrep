# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/micahkepe/jsongrep/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/micahkepe/jsongrep/releases/tag/v0.2.0
[0.1.2]: https://github.com/micahkepe/jsongrep/releases/tag/v0.1.2
[0.1.1]: https://github.com/micahkepe/jsongrep/releases/tag/v0.1.1
