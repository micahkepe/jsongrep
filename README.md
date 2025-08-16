# jsongrep (`jg`)

`jsongrep` is a JSONPath-inspired query language over JSON documents.

## Table of Contents

- [Installation](#installation)
- [Usage](#usage)
  - [Query Syntax](#query-syntax)
- [Examples](#examples)
- [Shell Completions](#shell-completions)
- [Man Page](#man-page)
- [Contributing](#contributing)
- [License](#license)

## Installation

`jsongrep` can be installed using `cargo`:

```bash
cargo install jsongrep
```

The `jg` binary will be installed to `~/.cargo/bin`.

## Usage

```
A JSONPath-inspired query language for JSON documents

Usage: jg [OPTIONS] [QUERY] [FILE] [COMMAND]

Commands:
  generate  Generate additional documentation and/or completions

Arguments:
  [QUERY]  Query string (e.g., "**.name")
  [FILE]   Optional path to JSON file. If omitted, reads from STDIN

Options:
      --compact     Do not pretty-print the JSON output, instead use compact
      --count       Display count of number of matches
      --depth       Display depth of the input document
  -n, --no-display  Do not display matched JSON values
  -h, --help        Print help
  -V, --version     Print version
```

### Query Syntax

The query engine allows you to query JSON data using a simple DSL. It supports
the following operators:

- Field accesses: `foo`
- Array accesses (0-indexed): `[0] | [start: end]`
- Field and array wild cards: `foo.*`, `foo[*]`
- Optional chaining: `foo?.bar`
- Kleene star: `foo*`
- Disjunction: `foo | bar`
- Sequence: `foo.bar.baz`

**Notes**:

- Sequences use `.` to chain steps: `foo.bar.baz`

- Fields can be unquoted (`foo`) or quoted (`"foo bar"`)

- The `*` modifier after a step is different from the `*` field wildcard — the
  modifier repeats **the preceding step**.

The complete grammar for the query language can be found in the
[grammar](./src/query/grammar) directory.

---

<details>
<summary>CLI</summary>

**Example**: Pass input file by path

`simple.json`:

```json
{
  "name": {
    "first": "John",
    "last": "Doe"
  },
  "age": 32,
  "hobbies": ["fishing", "yoga"]
}
```

The following query will follow an arbitrary amount of filed accesses followed
by a wildcard array access:

```bash
jg "**.[*]" simple.json
```

Output:

```text
[
  "fishing",
  "yoga"
]
```

**Example**: Pipe input from STDIN

```bash
curl https://api.nobelprize.org/v1/prize.json | jg "prizes[4].laureates[1].motivation"
```

Output:

```text
[
  "\"for foundational discoveries and inventions that enable machine learning with artificial neural networks\""
]
```

**Example**: Check number of matches without displaying them

Again, using the `simple.json` file:

```bash
jg "**.[*]" simple.json --count --no-display
```

Output:

```text
Found matches: 2
```

</details>

---

<details>
<summary>Rust API: QueryBuilder</summary>

The `jsongrep::query::ast` module defines the `QueryBuilder` fluent API for
building queries. It allows you to construct queries using a builder pattern.

**Example Usage**:

```rust
// Construct the query "foo[0].bar.*.baz"
use jsongrep::query::engine::QueryBuilder;
let query = QueryBuilder::new()
    .field("foo")
    .index(0)
    .field("bar")
    .field_wildcard()
    .field("baz")
    .build();
```

</details>

## Examples

Examples of using the `jsongrep` crate can be found in the
[examples](./examples) directory.

## Shell Completions

To generate completions for your shell, you can use the `jg generate shell`
subcommand. By default, the completions will be printed to `/dev/stdout` and can
be redirected to your shell's expected completion location:

- Bash

  ```bash
  # Source the completion script in your .bashrc
  echo 'source /path/to/jg.bash' >> ~/.bashrc

  # Or copy to the system-wide bash completion directory
  jg generate shell bash > jg.bash && sudo mv jg.bash /etc/bash_completion.d/
  ```

- Zsh

  ```bash
  mkdir -p ~/.zsh/completions
  jg generate shell zsh > ~/.zsh/completions/_jg

  # Add the directory to fpath in your .zshrc before compinit
  echo 'fpath=(~/.zsh/completions $fpath)' >> ~/.zshrc
  echo 'autoload -Uz compinit && compinit' >> ~/.zshrc
  ```

- Fish

  ```bash
  jg generate shell fish > ~/.config/fish/completions/jg.fish
  ```

## Man Page

To generate a Man page for `jg`, you can use the `jg generate man` subcommand to
generate to a specified output directory with the `-o`/`--output-dir` options
(defaults to current directory):

```bash
mkdir -p ~/.local/share/man/man1/
jg generate man -o ~/.local/share/man/man1/
```

Browse the generated Man pages with `man jg`.

## Contributing

Contributions are welcome! Please see the [CONTRIBUTING.md](CONTRIBUTING.md)
file for more details.

## License

This project is licensed under the MIT License - see the
[LICENSE.md](LICENSE.md) file for details.
