# See: <https://just.systems/man/en/>
update-submodules:
    git submodule update --init --recursive --remote

bench:
    cargo bench --bench query
