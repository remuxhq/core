# Typing discipline

The compiler is the gate: `make remuxd.check` runs clippy with `-D warnings` and the
tests.

- Illegal states unrepresentable: enums over booleans and strings (`Shape`, `Filter`,
  `Loaded`, `Polled`), tagged unions over flag-and-null.
- A secret is a `destinations::Key`: `Debug` prints stars, serde carries it. Never a
  bare `String` for a key or a token.
- The wire (`protocol.rs`) is serde with `#[serde(default)]` on every field added after
  a face shipped; a contract test asserts exact bytes. `schemars` derives the schema
  `remux schema` prints.
- Ports are traits with defaults for what a motor may not have (`Air::road`,
  `Watching::announce_armed`); a fake in the tests records calls.
- Clippy as errors, `gofmt`-style: `cargo fmt --all` before every commit.
