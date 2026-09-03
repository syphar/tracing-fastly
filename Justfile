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
    cargo clippy --all -- -Dwarnings
    actionlint
    cargo machete
    cargo audit

test:
    cargo nextest run
    cargo test --doc
