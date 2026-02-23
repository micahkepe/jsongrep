# See: <https://just.systems/man/en/>
update-submodules:
    git submodule update --init --recursive --remote

bench:
    cargo bench --bench query

bench-download:
    mkdir -p benches/data
    curl -Lo benches/data/citylots.json \
        https://raw.githubusercontent.com/zemirco/sf-city-lots-json/master/citylots.json

bench-publish:
    @echo "Publishing criterion results to gh-pages..."
    ghp-import -n -p -f target/criterion
