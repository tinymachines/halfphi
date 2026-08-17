# Assets

The mark: a wave breaking off a DIP package. Half of phi, coming out of the
silicon.

| file | size | what it is for |
|---|---|---|
| `halfphi.png` | 1408 x 1408 | the original render, for anything printed or large |
| `halfphi-512.png` | 512 x 512 | what the README shows, at 180px wide |
| `halfphi-180.png` | 180 x 180 | exact-size copy, for anywhere that will not scale |

The README pulls the 512 rather than the original: it is displayed at 180px
wide, and shipping two megabytes for that on every page view buys nothing.

## Two things to know before reusing it

The dark background is **baked in**, not transparent. On GitHub's light theme it
reads as a dark tile, which looks deliberate; anywhere it needs to sit on a light
surface, it needs a cut-out version instead. The background is a subtle gradient
rather than a flat colour, so that means masking the subject rather than a flood
fill.

There is **no vector version**. It is a render, not a drawing, so scaling past
1408px means re-rendering rather than exporting.
