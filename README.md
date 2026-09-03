# tracing-fastly

`tracing-fastly` sends structured [`tracing`](https://docs.rs/tracing) events
from Fastly Compute applications to logging endpoints.

The crate provides a `tracing-subscriber` layer that combines every event with
the fields of its active spans. The resulting log record is self-contained, so
fields such as `request_id`, `backend`, or `route` can be queried without
knowing whether they originated on the event or a request span. Event fields
take precedence over the innermost span, followed by its ancestors.

Included are:

- a Datadog sink using Datadog's reserved JSON attributes;
- generic NDJSON writing and serialization helpers for custom providers; and
- an optional `testing` feature with a writer for provider tests.

## Datadog

Configure a Fastly logging endpoint, then add the structured layer to the
subscriber:

```rust
use fastly::log::Endpoint;
use tracing_fastly::{StructuredEventLayer, providers::datadog};
use tracing_subscriber::{filter::LevelFilter, prelude::*};

let endpoint = Endpoint::from_name("trace_logs");

tracing_subscriber::registry()
    .with(
        StructuredEventLayer::new(
            datadog::TraceSink::new(endpoint, "example_service")
                .with_tags("env:production,version:1.0"),
        )
        .with_filter(LevelFilter::INFO),
    )
    .init();
```

Fields on an enclosing request span are included in each event:

```rust
let request = tracing::info_span!(
    "request",
    request_id = "req-abc-123",
    route = "/docs",
);
let _guard = request.enter();

tracing::info!(status = 200, backend = "origin", "handled request");
```

A regular `tracing-subscriber` formatting layer can be installed alongside it
for human-readable output in `fastly log-tail`. See
[`examples/trace_datadog.rs`](examples/trace_datadog.rs) for the complete setup.

## Custom providers

Implement `StructuredEventSink` to define another provider's exact wire format.
`StructuredEvent` exposes the timestamp, level, message, and combined event/span
fields. `serialize::NdjsonWriter` serializes a row and sends it to the
underlying Fastly endpoint with one `Write::write` call, preserving Fastly's
log-record boundary.

See [`examples/custom_provider.rs`](examples/custom_provider.rs) for a complete
implementation and provider test. Enable the `testing` feature in a development
dependency to use `testing::RecordWriter` in downstream tests.

## Development

Rust is managed with `rustup`; the remaining development tools are pinned in
[`mise.toml`](mise.toml). Install the WASI target and project tools with:

```console
rustup target add wasm32-wasip1
mise install
```

Common commands are defined in the [`Justfile`](Justfile):

```console
just format-check  # verify Rust, Justfile, Cargo.toml, and Markdown formatting
just lint          # Clippy, workflow, action pinning, dependency, and audit checks
just test          # release-mode WASI tests executed through Viceroy
```

`just lint` lets `pinact` update GitHub Action pins locally; in CI it uses check
mode and fails when the workflow is not pinned. Viceroy reads the local runtime
configuration from [`fastly.toml`](fastly.toml). The crate's default Cargo
target is `wasm32-wasip1`, as configured in
[`.cargo/config.toml`](.cargo/config.toml).
