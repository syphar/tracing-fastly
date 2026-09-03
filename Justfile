lint: 
  cargo fmt --check
  cargo clippy --all -- -Dwarnings
  cargo sort
