#! /bin/env bash

wkg wit fetch
cargo build --release --target wasm32-wasip2
