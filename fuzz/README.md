# Fuzzing

`cargo test -p bach-pack --test robustness` damages valid input thousands of
ways with a fixed seed and runs in every `cargo test`
(`BACH_FUZZ_ITERATIONS=20000` makes it longer). These cargo-fuzz targets are
for long runs with coverage guidance:

```bash
cargo install cargo-fuzz        # needs a nightly toolchain
cd fuzz
cargo +nightly fuzz run archive # or manifest, feed, config
```

The targets are not built by CI and were not run when they were written.
