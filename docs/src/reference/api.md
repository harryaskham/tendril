# Rust API docs

The published site includes generated rustdoc for the workspace at `api/`.

## Published location

- site-relative path: `api/index.html`
- local preview path: `target/book/api/index.html`

## What is included

The current Pages build copies `cargo doc --workspace --no-deps` output into the published artifact, which covers:

- `tendril`
- `mcp-cli`

## Why this is separate from the guide content

Narrative docs explain how to use Tendril as a product. Rust API docs explain crate-level types and modules. Publishing both together gives the repository a stable home for usage guides, reference pages, and generated API documentation without splitting the experience across multiple tools.
