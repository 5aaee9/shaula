## Lint

Make sure pass the clippy and rustfmt test via: cargo clippy and cargo fmt after edit files.

Split rust modules into multiple files when files grow too large. Start split when file exceed 400 line of file (LOC). DO NOT use `include_bytes!` macro to bypass this limit.
Try not to use #[allow] to bypass limit.

Run cargo nextest run --manifest-path "Cargo.toml" --workspace test after task done.
