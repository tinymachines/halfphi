# Notices and licensing

Read this before redistributing a build.

## This crate

MIT, copyright Tiny Machines — see `LICENSE`.

## It contains no die data, deliberately

`halfphi` reads chip descriptions; it does not include any. That is the whole
reason it can be MIT.

The die traces published by the [visual6502](https://github.com/trebonian/visual6502)
project — `segdefs.js` and `transdefs.js`, the polygon and transistor geometry —
are licensed **CC BY-NC-SA 3.0**:

    Copyright (c) 2010 Greg James, Brian Silverman, Barry Silverman
    http://creativecommons.org/licenses/by-nc-sa/3.0/

**NonCommercial and ShareAlike propagate to anything that ships that data**, or
anything derived from it: a netlist blob, a geometry blob, a binary or `.wasm`
that embeds either, and any deployed application built from them.

This crate never becomes that thing on its own. If *you* ship a build that
embeds a die trace, those terms are yours to honour. Commercial use would need
geometry re-derived from an independent trace, or separate permission from the
rights holders.

`extern/visual6502` here is a git submodule, not a copy, so this repository does
not redistribute the data at all — it points at the project that licensed it.

## Portions derived from visual6502 — MIT

The switch-level solver is a Rust port of the corresponding JavaScript in
visual6502 (`chipsim.js`, `wires.js`). That code is MIT, and its notice is
reproduced in `LICENSE-THIRD-PARTY`. It is a different licence grant from the
die data above, by different authors, and the distinction is the reason this
crate can exist.
