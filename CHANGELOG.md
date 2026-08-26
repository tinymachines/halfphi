# Changelog

Notable changes. This project follows [semantic versioning](https://semver.org),
with the caveat that `0.x` means the API is still moving.

## [Unreleased]

## [0.1.1] - 2026-08-26

### Fixed

- `Engine::recalc_node` wrote a group's resolved level into a rail that was a
  member of the group, so a group joined to both rails left vcc's stored level
  low. Unobservable to the solver, which resolves a rail by identity rather
  than by state, but visible to anything reading levels out: on the 6502 vcc
  dipped for half a cycle on most opcode fetches, and on the Z80 the bounce
  switched the 32 supply-gated transistors and stopped a cold power-on from
  converging. Rails are never written now. Found by a reader of a node-level
  export, not by the differential test, which masks the rails by construction.

### Changed

- The Z80 converges from a cold power-on. `tests/chips.rs` records it, and the
  README's "untested lead" about supply-gated transistors is now the account.

## [0.1.0]

First release. Extracted from [tinymachines/6502](https://github.com/tinymachines/6502),
where it had been the chip-agnostic core of a 6502 simulator without being
separable from it.

### Added

- `source::parse` — reads a visual6502-format die trace into a netlist blob plus
  geometry. Previously lived in a build script, where nothing could call it.
- `Netlist` — immutable topology with CSR adjacency, decoded from that blob.
- `Engine` — the switch-level solver: groups, drive resolution, settling.
- Power rails as a parameter (`Rails`), because the 6800 calls ground `gnd`.
- Boolean and null literals in the parser: the 6800's `transdefs` carry a
  seventh field per transistor that the 6502's do not.
- Tests across three dies — 6502, 6800, Z80 — through identical calls.

### Known

- The Z80 does not reach a fixed point from a cold power-on within the hundred
  rounds the reference implementation also capped at. Recorded in the tests as
  an expectation. No chip-specific initialisation is performed, and visual6502
  ships one per chip.
- 6800 and Z80 figures are checked for shape only, not against any outside
  source.
- The Z80 has 32 transistors gated by its supply rail, which this model treats
  as permanently off where silicon treats them as permanently on. Reported by
  the `inspect` example. Possibly related to the above; not investigated.

[Unreleased]: https://github.com/tinymachines/halfphi/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/tinymachines/halfphi/releases/tag/v0.1.1
[0.1.0]: https://github.com/tinymachines/halfphi/releases/tag/v0.1.0
