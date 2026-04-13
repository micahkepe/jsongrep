# JSONGrep Playground

A browser-based playground for
[jsongrep](https://github.com/micahkepe/jsongrep), powered by WebAssembly.

## Prerequisites

- [Bun](https://bun.sh)
- [Rust](https://rustup.rs) with the `wasm32-wasip2` target:

  ```bash
  rustup target add wasm32-wasip2
  ```

## Development

```bash
bun install
bun dev
```

## Production build

```bash
bun run build
```

The output is written to `dist/` and can be served as a static site.
