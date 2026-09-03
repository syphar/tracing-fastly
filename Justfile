TARGET := "wasm32-wasip1"
TARGET_UPPER := shoutysnakecase(TARGET)

format-check: (format "--check")

format check="":
    cargo fmt {{ check }}
    just --fmt {{ check }}
    cargo sort {{ check }}
    fd \
      --type file \
      --threads 1 \
      --extension md \
      --exec deno fmt --quiet {{ check }}

lint:
    cargo clippy --locked -- -D warnings
    actionlint
    cargo machete
    cargo audit

test:
    # needs `cargo binstall viceroy cargo-nextest`
    CARGO_TARGET_{{ TARGET_UPPER }}_RUNNER="viceroy run -C fastly.toml --" \
      cargo nextest run --release --target {{ TARGET }} 
