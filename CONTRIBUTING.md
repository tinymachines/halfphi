# Contributing

## The one rule

**Measure before you claim.** This library exists because behaviour falls out of
simulating switches rather than being written down, and the same standard
applies to anything said about it. A number in a comment, a README or a doc
string should be reproducible by running something. If it is not checkable,
write down that it is not.

That cuts both ways: a result that is inconvenient gets recorded rather than
asserted away. The Z80 not converging from a cold power-on is in the test suite
as an expectation, with an explanation of what it does and does not mean, and
that is the pattern to follow.

## Ground rules for code

- **No die data in this repository.** Ever. It is the reason the crate is MIT.
  Test data comes from the submodule; if you need a new chip, add it there or
  point the tests at a path.
- **No dependencies** without a strong argument. There are currently zero, which
  makes this crate cheap to audit and cheap to depend on.
- **Nothing may name a chip.** If you find yourself writing `"vss"`, `"clk"` or
  `"idb0"` in `src/`, that fact belongs in a chip layer, not here. Rails are
  already a parameter for exactly this reason.
- **Faithfulness beats tidiness.** Several things in the parser and the solver
  look like cleanup opportunities and are not: the pullup flag comes from the
  first polygon mentioning a node rather than the OR of all of them, and
  terminal normalisation is two sequential ifs where the second sees the first's
  result. Both are ported deliberately. If you change one, the 6502 netlist stops
  matching the reference bit for bit and you will not notice from the tests here.

## Before opening a PR

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test -- --nocapture
```

If you touched the parser or the solver, say so in the PR: those two have a
downstream consumer ([tinymachines/6502](https://github.com/tinymachines/6502))
that checks them against the original JavaScript engine bit-exactly over 3000
half-cycles, which is a stronger test than anything in this repository.

## Adding a chip

The visual6502 collection publishes each die as `segdefs.js`, `transdefs.js` and
`nodenames.js`. Adding one to `tests/chips.rs` means giving its rail names and
saying whether it converges from a cold power-on. Please do not assert figures
for a chip that has not been checked against an outside source — mark them as
shape-only, as the 6800 and Z80 currently are.
