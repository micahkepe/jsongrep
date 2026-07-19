# See: <https://just.systems/man/en/>
set default-list := true

# Update all submodules.
update-submodules:
    git submodule update --init --recursive --remote

# Run Criterion benchmarks.
bench: bench-download
    cargo bench --bench query

# Download data for the Criterion benchmarks.
bench-download:
  #!/usr/bin/env bash
  mkdir -p benches/data
  if [[ ! -f "{{justfile_directory()}}/benches/data/citylots.json" ]]; then
    curl -Lo benches/data/citylots.json \
      https://raw.githubusercontent.com/zemirco/sf-city-lots-json/master/citylots.json
  fi

# Publish Criterion results to gh-pages
bench-publish:
    @echo "Publishing criterion results to gh-pages..."
    ghp-import -n -p -f target/criterion
