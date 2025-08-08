# rq

`rq` is a JSONPath-inspired query language over JSON documents.

## Usage

```
Query an input JSON document against a rq query

Usage: rq [OPTIONS] <QUERY> [FILE]

Arguments:
  <QUERY>  Query string (e.g., "**.name")
  [FILE]   Optional path to JSON file. If omitted, reads from STDIN

Options:
      --compact     Do not pretty-print the JSON output, instead use compact
      --count       Display count of number of matches
      --depth       Display depth of the input document
  -n, --no-display  Do not display matched JSON values
  -h, --help        Print help
  -V, --version     Print version
```

## License

This project is licensed under the MIT License - see the
[LICENSE.md](LICENSE.md) file for details.
