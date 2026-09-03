# Changelog

Notable changes. This project follows [semantic versioning](https://semver.org),
with the caveat that `0.x` means the API is still moving.

## [Unreleased]

### Changed

- The rail-conflict hold's fallback is now the reference's own rule in
  full: pulls first, then an AREA-WEIGHTED charge vote over the group's
  members, with the weights supplied through the new
  `Netlist::set_node_areas` (required before marking holds, because a
  vote without weights is a different rule wearing this one's name).
  0.1.4's fallback counted charge instead of weighing it, and two
  variants of that died on the 2C02's P0 golden at init; the
  instrumented reference showed why, eleven charged members losing the
  vote to two large ones. With the area vote the 2C02's P0 and P1
  goldens replay green, and the sprite-0 scenario that exposed the whole
  path matches the reference at every checkpoint: the x position counter
  loads, counts down, and the hit lands at the sprite's authored x.
  Inert wherever no holds are marked, asserted zero on the 6502, the
  6800 and the Z80.

## [0.1.4] - 2026-09-03

### Added

- The rail-conflict hold: `Netlist::set_rail_conflict_holds` marks nodes
  whose groups, when joined to BOTH rails at once, resolve by their pulls
  and held charge instead of by either rail, with
  `Stats::rail_conflict_holds` counting each application. Chip-agnostic
  mechanism, chip-supplied list, the same shape as `Rails`: the 2C02's
  OAM data lines forced it. Its reference simulator special-cases exactly
  those eight nodes when a group holds both rails; without the hold, this
  engine resolved such groups Vss-wins and crushed the sprite x byte to
  zero on its way to the position counters, so sprite 0 rendered at the
  left edge while the reference rendered it at its authored x (both
  measured, same script, same checkpoints). An empty list, which every
  other chip passes, changes nothing: the both-rails fold is bit-for-bit
  the old behaviour, and the chips test now asserts the count stays zero
  on the 6502, the 6800 and the Z80.

## [0.1.3] - 2026-09-03

### Changed

- `Drive`'s order between the two pulls swapped: `PullDown` now outranks
  `PullUp` (`Floating < ChargedHigh < PullUp < PullDown < Vcc < Vss`), and
  `slice`'s thermometer planes swapped in step. Decided by measurement, not
  taste: a group holding both pulls is a layout depletion load fighting an
  external drive or an init-forced level, and the stronger side wins low.
  The fifth chip through these calls, the 2A03, is the first to form such
  groups (three, its SO input chain, at power-on: Quietust's init drives
  `so` low while the group carries a layout pullup); its reference resolves
  them low by first-match from the driven seed, the old order resolved them
  high, and with the swap its 601-state node golden replays bit-exact with
  no exemptions. On the 6502, 6800, Z80 and 2C02 the order is unobservable:
  `Stats::contested_groups` is 0 on all of them, now asserted in the chips
  test, and the 6502 workspace's full suite (the node golden, the pin
  golden, rungs 1 through 3 and the slice-based engines) was re-proven
  green on this change with the goldens required.

## [0.1.2] - 2026-08-27

### Added

- `slice` — the solver as a kernel: 64 machines in one instruction stream, one
  per bit of a `u64`. The drive lattice is encoded as a thermometer so that
  taking the maximum becomes a bitwise OR, and "is this transistor conducting"
  becomes a mask rather than a branch, so no lane needs its own control path.
  Measured on the 6502 at 2.5x `Engine`'s machine-half-cycles per second while
  sweeping every transistor every round.

  **It is not bit-exact with `Engine`, by nature rather than by defect.** This
  hardware stores charge, so a node briefly joined to a driver keeps that level
  after the switch reopens: the settled state depends on the path taken and not
  only on the final switch configuration. A queue stages a specific sequence of
  configurations, including momentary ones; a lane-uniform sweep cannot, because
  queue order is data dependent and therefore lane dependent. Agreement is at
  the level of program results, not trajectories: on the 6502's `INC`/`JMP`
  loop, identical memory after 3000 half-cycles, 2061 of those half-cycles
  identical on all 1702 live nodes, worst case 2 nodes differing. Use `Engine`
  when the trajectory matters, which includes anything checked against the
  reference JavaScript engine.

- `BitSet::insert_if` — set a bit only if a condition holds, returning whether
  this call set it, in one load and one store and without branching on either.

### Changed

- `Engine::build_group` appends branchlessly. Its two inner conditions are
  unpredictable by construction, because which switches conduct is the thing
  being simulated; the loop now writes each candidate and advances the length
  by a bool. Measured on the 6502, interleaved A/B: **39% fewer branch
  mispredictions** (470M to 287M), 17.7% more instructions, 12% fewer cycles,
  16.9% more throughput best-of-three. Bit-exact against the reference engine
  over every node at every half-cycle, and unchanged for the 6800 and the Z80.
- The group buffer is now a fixed allocation of `node_count + 1` with its
  length carried alongside, because a `Vec` grown by `push` cannot be filled
  without a branch. Still no `unsafe`.

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

[Unreleased]: https://github.com/tinymachines/halfphi/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/tinymachines/halfphi/releases/tag/v0.1.4
[0.1.3]: https://github.com/tinymachines/halfphi/releases/tag/v0.1.3
[0.1.2]: https://github.com/tinymachines/halfphi/releases/tag/v0.1.2
[0.1.1]: https://github.com/tinymachines/halfphi/releases/tag/v0.1.1
[0.1.0]: https://github.com/tinymachines/halfphi/releases/tag/v0.1.0
