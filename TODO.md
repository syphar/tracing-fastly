# TODO

- use human-readable timestamp for the UI , see crates.io static
- add tracing target
- `ddsource` must be something that idendifies a standard format? or a custom
  nam?

I’d add only module_path next, if you want the actual Rust module:

module: event.metadata().module_path(),

target is usually the module path, but callers can override it—our new test does
exactly that—so they are not synonymous.

I would not add these by default:

- name: for tracing events it is generally compiler-generated and less useful
  than message.
- file and line: useful for debugging, but high-cardinality/noisy in Datadog.
  Consider adding them only to error logs.
- Thread/process metadata: not useful for this Compute worker.
- Trace/span IDs: only when introducing actual OpenTelemetry/APM trace
  propagation; request_id should remain separate.

The other useful context is span names. Currently we merge span fields but
discard names such as request and operation. A span or span_path field could
make nested logs easier to interpret, though it adds some per-event allocation.
