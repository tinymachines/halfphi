## What this changes

<!-- One or two sentences. -->

## How it was checked

<!-- What did you run, and what did it say? A number here beats a description.
     If you changed the parser or the solver, note it: they have a downstream
     consumer that checks them bit-exactly against the original engine. -->

- [ ] `cargo test` passes
- [ ] `cargo clippy --all-targets -- -D warnings` is clean
- [ ] `cargo fmt --all -- --check` is clean
- [ ] No die data was added to this repository
- [ ] Nothing in `src/` names a particular chip
