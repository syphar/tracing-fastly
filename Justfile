format check="":
    cargo fmt {{ check }}
    just --fmt {{ check }}
    cargo sort {{ check }}

lint:
    cargo clippy --all -- -Dwarnings
    cargo machete
    cargo audit
