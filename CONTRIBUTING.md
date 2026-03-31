# Contributing

Thanks for helping improve **jsongrep**!

## How to Contribute

- **Bugs** &rarr; Open an issue with steps to reproduce, expected vs. actual
  behavior, and your `jsongrep` version (`jg --version`).
- **Features** &rarr; Open an issue tagged **enhancement** describing the
  problem and your proposed solution.
- **Pull Requests** &rarr; Fork &rarr; Branch &rarr; Code &rarr; Test &rarr; PR.

## Style Guidelines

- Run `cargo fmt` to follow the project's `rustfmt.toml`.
- Document public items with `///` doc comments.
- Prefer small, focused tests named after the scenario they cover.

## PR Checklist

- [ ] Clear, focused commits ([Conventional
      Commits](https://www.conventionalcommits.org/en/v1.0.0/) please!)
- [ ] Tests added/updated if needed
- [ ] Code passes `cargo fmt` + `cargo clippy`
- [ ] Update `CHANGELOG.md` under `[Unreleased]` if user-facing
