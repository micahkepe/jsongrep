- `examples/query_builder.rs` has a wrong assertion: expects `**` for
  `field_wildcard()` but it correctly produces `*`. The assertion string should
  be `"bar[2]? | foo[2:5].*.[*]"`.

- Library API leaks `serde_json_borrow::Value` — users must depend on it
  directly. Should either re-export it from the crate or provide a convenience
  method that accepts `&str` / `serde_json::Value` directly.

- YAML parser is extremely permissive — feeding TOML (or most any text) with
  `--format yaml` silently parses without error, producing unexpected structure.
  Zero query matches on a misidentified format is indistinguishable from zero
  matches on correct input. Not a bug, but a UX wart worth documenting.
