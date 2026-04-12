#! /bin/env bash
#
set -xe

wasmtime run --invoke 'query("{\"okay\": 1}", "okay")' ./target/wasm32-wasip2/release/jsongrep_wasm.wasm | sed 's/^ok(//; s/)$//'
wasmtime run --invoke 'query-first("{\"okay\": 1}", "okay")' ./target/wasm32-wasip2/release/jsongrep_wasm.wasm | sed 's/^ok(//; s/)$//' | jq -r
wasmtime run --invoke 'query-with-path("{\"okay\": 1}", "okay")' ./target/wasm32-wasip2/release/jsongrep_wasm.wasm | sed 's/^ok(//; s/)$//'
wasmtime run --invoke 'query-with-timings("{\"okay\": 1}", "okay")' ./target/wasm32-wasip2/release/jsongrep_wasm.wasm | sed 's/^ok(//; s/)$//'

# yaml support
wasmtime run --invoke 'query("okay: 1", "okay")' ./target/wasm32-wasip2/release/jsongrep_wasm.wasm | sed 's/^ok(//; s/)$//'
wasmtime run --invoke 'query-first("okay: 1", "okay")' ./target/wasm32-wasip2/release/jsongrep_wasm.wasm | sed 's/^ok(//; s/)$//'
wasmtime run --invoke 'query-with-path("okay: 1", "okay")' ./target/wasm32-wasip2/release/jsongrep_wasm.wasm | sed 's/^ok(//; s/)$//'
wasmtime run --invoke 'query-with-timings("okay: 1", "okay")' ./target/wasm32-wasip2/release/jsongrep_wasm.wasm | sed 's/^ok(//; s/)$//'
