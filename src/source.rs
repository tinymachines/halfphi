//! Reading a photographed die into a netlist.
//!
//! The visual6502 project publishes each chip it has traced as three JavaScript
//! files -- `segdefs.js` (the polygons, and which node each belongs to),
//! `transdefs.js` (the transistors) and `nodenames.js` (the names) -- and every
//! chip in that collection uses the same three, in the same shapes. That is the
//! whole reason this library can be about more than one chip.
//!
//! This parser used to live in a build script, where nothing could call it. It
//! is the same code; moving it here is what turns "we can build one netlist at
//! compile time" into "anything can build a netlist from any of these dies".
//!
//! Nothing here embeds any die data. The data is CC BY-NC-SA and that licence
//! propagates to whatever ships it; this crate stays MIT by holding none of it,
//! and asking the caller for the bytes instead. See NOTICE.md.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Val {
    Num(i64),
    Str(String),
    Arr(Vec<Val>),
    Obj(Vec<(String, Val)>),
    Bool(bool),
    Null,
}

impl Val {
    pub fn as_num(&self) -> Option<i64> {
        match self {
            Val::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&[Val]> {
        match self {
            Val::Arr(a) => Some(a),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser {
            s: s.as_bytes(),
            i: 0,
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.s[self.i..].starts_with(b"/*") {
                self.i += 2;
                while self.i < self.s.len() && !self.s[self.i..].starts_with(b"*/") {
                    self.i += 1;
                }
                self.i = (self.i + 2).min(self.s.len());
            } else {
                return;
            }
        }
    }

    fn peek(&mut self) -> u8 {
        self.skip_trivia();
        if self.i < self.s.len() {
            self.s[self.i]
        } else {
            0
        }
    }

    fn parse_value(&mut self) -> Result<Val, String> {
        match self.peek() {
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'\'' | b'"' => Ok(Val::Str(self.parse_string()?)),
            c if c == b'-' || c == b'+' || c.is_ascii_digit() => self.parse_number(),
            // Booleans and null. The 6502's data contains neither, so this
            // parser did without them for the life of the project -- and then
            // the 6800's transdefs turned out to carry a seventh field per
            // transistor, a bare `false`. A parser written against one chip is
            // the first thing to break on the second one, and it fails at a byte
            // offset rather than anywhere meaningful.
            b't' | b'f' | b'n' => self.parse_keyword(),
            c => Err(format!(
                "unexpected byte {:?} at offset {}",
                c as char, self.i
            )),
        }
    }

    fn parse_keyword(&mut self) -> Result<Val, String> {
        for (word, val) in [
            ("true", Val::Bool(true)),
            ("false", Val::Bool(false)),
            ("null", Val::Null),
        ] {
            if self.s[self.i..].starts_with(word.as_bytes()) {
                self.i += word.len();
                return Ok(val);
            }
        }
        Err(format!("unknown keyword at offset {}", self.i))
    }

    fn parse_array(&mut self) -> Result<Val, String> {
        self.i += 1; // '['
        let mut out = Vec::new();
        loop {
            if self.peek() == b']' {
                self.i += 1;
                return Ok(Val::Arr(out));
            }
            out.push(self.parse_value()?);
            match self.peek() {
                b',' => self.i += 1,
                b']' => {}
                c => return Err(format!("expected , or ] got {:?} at {}", c as char, self.i)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<Val, String> {
        self.i += 1; // '{'
        let mut out = Vec::new();
        loop {
            if self.peek() == b'}' {
                self.i += 1;
                return Ok(Val::Obj(out));
            }
            let key = match self.peek() {
                b'\'' | b'"' => self.parse_string()?,
                _ => {
                    let start = self.i;
                    while self.i < self.s.len()
                        && (self.s[self.i].is_ascii_alphanumeric()
                            || self.s[self.i] == b'_'
                            || self.s[self.i] == b'$')
                    {
                        self.i += 1;
                    }
                    if start == self.i {
                        return Err(format!("empty object key at {}", self.i));
                    }
                    String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
                }
            };
            if self.peek() != b':' {
                return Err(format!("expected : after key {key:?} at {}", self.i));
            }
            self.i += 1;
            let value = self.parse_value()?;
            out.push((key, value));
            match self.peek() {
                b',' => self.i += 1,
                b'}' => {}
                c => {
                    return Err(format!(
                        "expected , or }} got {:?} at {}",
                        c as char, self.i
                    ))
                }
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        let quote = self.s[self.i];
        self.i += 1;
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] != quote {
            self.i += 1;
        }
        let out = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
        self.i += 1; // closing quote
        Ok(out)
    }

    fn parse_number(&mut self) -> Result<Val, String> {
        let start = self.i;
        if self.s[self.i] == b'-' || self.s[self.i] == b'+' {
            self.i += 1;
        }
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
            self.i += 1;
        }
        let text = String::from_utf8_lossy(&self.s[start..self.i]);
        // The data files are all-integer; tolerate a trailing ".0" defensively.
        let text = text.split('.').next().unwrap_or("");
        text.parse::<i64>()
            .map(Val::Num)
            .map_err(|e| format!("bad number {text:?} at {start}: {e}"))
    }
}

/// Find `var <name> = <value>` (or `<name> = <value>`) and parse the literal.
pub fn parse_decl(src: &str, name: &str) -> Result<Val, String> {
    let pat = format!("{name} =");
    let alt = format!("{name}=");
    let at = src
        .find(&pat)
        .map(|p| p + pat.len())
        .or_else(|| src.find(&alt).map(|p| p + alt.len()))
        .ok_or_else(|| format!("declaration of `{name}` not found"))?;
    let mut p = Parser::new(&src[at..]);
    p.parse_value()
}

/// The netlist blob's magic. Not `V6502NL1` any more: the format describes a
/// switch network, and nothing in it is about one chip.
const MAGIC: &[u8; 8] = b"HALFPHI1";

struct Blob(Vec<u8>);

impl Blob {
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bits(&mut self, flags: &[bool]) {
        for chunk in flags.chunks(8) {
            let mut byte = 0u8;
            for (i, &b) in chunk.iter().enumerate() {
                if b {
                    byte |= 1 << i;
                }
            }
            self.0.push(byte);
        }
    }
}

// ---------------------------------------------------------------------------
// The chip source: three files in, one netlist blob out
// ---------------------------------------------------------------------------

/// Which node names are the power rails, in the order (ground, supply).
///
/// This is a parameter and not a constant because it genuinely varies: the 6502
/// and the Z80 call their rails `vss` and `vcc`, and the 6800 calls ground
/// `gnd`. Hardcoding the 6502's spelling is exactly the sort of thing that makes
/// a library about one chip while looking general.
#[derive(Debug, Clone, Copy)]
pub struct Rails<'a> {
    pub ground: &'a str,
    pub supply: &'a str,
}

impl Default for Rails<'_> {
    /// What most of the collection uses. The 6800 needs `gnd` for ground.
    fn default() -> Self {
        Rails {
            ground: "vss",
            supply: "vcc",
        }
    }
}

/// The three source files, as text.
pub struct ChipSource<'a> {
    pub segdefs: &'a str,
    pub transdefs: &'a str,
    pub nodenames: &'a str,
    pub rails: Rails<'a>,
}

/// One polygon on the die: which node owns it, which mask layer it is on.
///
/// Kept out of the netlist blob deliberately -- the simulation never needs
/// geometry, and a renderer needs nothing else. Callers that want to draw the
/// die triangulate these themselves.
#[derive(Debug, Clone)]
pub struct Polygon {
    pub layer: u8,
    pub node: u16,
    pub pts: Vec<(u16, u16)>,
}

/// What a parse yields: the encoded netlist, plus the geometry and the counts
/// that a caller may want to check or draw.
pub struct Parsed {
    /// The netlist, ready for `Netlist::decode`.
    pub blob: Vec<u8>,
    pub polygons: Vec<Polygon>,
    /// Gate bounding boxes `[xmin, xmax, ymin, ymax]`, one per transistor.
    pub gate_boxes: Vec<[u16; 4]>,
    pub node_count: usize,
    pub transistor_count: usize,
    pub name_count: usize,
    /// Transistors whose gate is tied to the supply rail. These are permanently
    /// *on* in silicon and permanently *off* in this model, because group
    /// evaluation never crosses a rail. None exist in the 6502; a nonzero count
    /// on another chip is a real divergence and not a curiosity.
    pub gated_by_supply: usize,
}

/// Parse one chip's three files into a netlist blob and its geometry.
///
/// Faithful to `wires.js:setupTransistors()` in two details that look like
/// tidying opportunities and are not: the pullup flag comes from the *first*
/// polygon that mentions a node rather than the OR of all of them, and terminal
/// normalisation runs as two sequential ifs where the second sees the first's
/// result.
pub fn parse(src: &ChipSource<'_>) -> Result<Parsed, String> {
    let nodenames = parse_decl(src.nodenames, "nodenames")?;
    let Val::Obj(entries) = &nodenames else {
        return Err("nodenames is not an object".into());
    };
    let mut names: Vec<(String, i32)> = Vec::with_capacity(entries.len());
    let mut name_to_node: HashMap<&str, i64> = HashMap::new();
    for (k, v) in entries {
        let n = v
            .as_num()
            .ok_or_else(|| format!("nodename {k} is not a number"))?;
        names.push((k.clone(), n as i32));
        name_to_node.insert(k.as_str(), n);
    }
    let rail = |which: &str| -> Result<u32, String> {
        name_to_node
            .get(which)
            .map(|n| *n as u32)
            .ok_or_else(|| format!("no rail called {which:?} in nodenames"))
    };
    let vss = rail(src.rails.ground)?;
    let vcc = rail(src.rails.supply)?;

    // segdefs: [node, '+'|'-' pullup, layer, x0,y0, x1,y1, ...]
    let segdefs = parse_decl(src.segdefs, "segdefs")?;
    let segs = segdefs.as_arr().ok_or("segdefs is not an array")?;
    let mut node_count = 0usize;
    for s in segs {
        let a = s.as_arr().ok_or("segdef entry is not an array")?;
        let n = a
            .first()
            .and_then(Val::as_num)
            .ok_or("segdef entry has no node number")? as usize;
        node_count = node_count.max(n + 1);
    }
    // A name may point at a node that owns no polygon; the array has to be big
    // enough to index by it either way.
    for (_, n) in &names {
        if *n >= 0 {
            node_count = node_count.max(*n as usize + 1);
        }
    }
    let mut exists = vec![false; node_count];
    let mut pullup = vec![false; node_count];
    let mut polygons: Vec<Polygon> = Vec::with_capacity(segs.len());
    for s in segs {
        let a = s.as_arr().unwrap();
        let n = a[0].as_num().unwrap() as usize;
        if !exists[n] {
            exists[n] = true;
            pullup[n] = a.get(1).and_then(Val::as_str) == Some("+");
        }
        let layer = a
            .get(2)
            .and_then(Val::as_num)
            .ok_or("segdef entry has no layer")? as u8;
        let coords = &a[3..];
        if coords.len() < 6 || coords.len() % 2 != 0 {
            continue; // fewer than 3 points cannot be filled
        }
        let mut pts = Vec::with_capacity(coords.len() / 2);
        for xy in coords.chunks_exact(2) {
            let x = xy[0].as_num().ok_or("non-numeric polygon x")?;
            let y = xy[1].as_num().ok_or("non-numeric polygon y")?;
            pts.push((x as u16, y as u16));
        }
        polygons.push(Polygon {
            layer,
            node: n as u16,
            pts,
        });
    }

    // transdefs: ['name', gate, c1, c2, [bbox], [geometry]]
    let transdefs = parse_decl(src.transdefs, "transdefs")?;
    let trans = transdefs.as_arr().ok_or("transdefs is not an array")?;
    let mut tg = Vec::with_capacity(trans.len());
    let mut gate_boxes: Vec<[u16; 4]> = Vec::with_capacity(trans.len());
    let mut gated_by_supply = 0usize;
    for t in trans {
        let a = t.as_arr().ok_or("transdef entry is not an array")?;
        if a.len() < 4 {
            return Err(format!("short transdef entry: {a:?}"));
        }
        let bb = a.get(4).and_then(Val::as_arr).unwrap_or(&[]);
        gate_boxes.push(if bb.len() >= 4 {
            [
                bb[0].as_num().unwrap_or(0) as u16,
                bb[1].as_num().unwrap_or(0) as u16,
                bb[2].as_num().unwrap_or(0) as u16,
                bb[3].as_num().unwrap_or(0) as u16,
            ]
        } else {
            [0; 4]
        });
        let gate = a[1].as_num().ok_or("transdef gate")? as u32;
        let mut c1 = a[2].as_num().ok_or("transdef c1")? as u32;
        let mut c2 = a[3].as_num().ok_or("transdef c2")? as u32;

        // Two sequential ifs, not exclusive: the second sees the first's result.
        if c1 == vss {
            c1 = c2;
            c2 = vss;
        }
        if c1 == vcc {
            c1 = c2;
            c2 = vcc;
        }
        if gate == vcc {
            gated_by_supply += 1;
        }
        for n in [gate, c1, c2] {
            if (n as usize) >= node_count || !exists[n as usize] {
                return Err(format!("transistor references unknown node {n}"));
            }
        }
        tg.push((gate as u16, c1 as u16, c2 as u16));
    }

    let mut b = Blob(Vec::with_capacity(1 << 16));
    b.0.extend_from_slice(MAGIC);
    b.u32(node_count as u32);
    b.u32(tg.len() as u32);
    b.u32(names.len() as u32);
    b.u32(vss);
    b.u32(vcc);
    b.bits(&exists);
    b.bits(&pullup);
    for (g, c1, c2) in &tg {
        b.u16(*g);
        b.u16(*c1);
        b.u16(*c2);
    }
    for (name, node) in &names {
        let bytes = name.as_bytes();
        b.u16(bytes.len() as u16);
        b.0.extend_from_slice(bytes);
        b.i32(*node);
    }

    Ok(Parsed {
        blob: b.0,
        polygons,
        gate_boxes,
        node_count,
        transistor_count: tg.len(),
        name_count: names.len(),
        gated_by_supply,
    })
}
