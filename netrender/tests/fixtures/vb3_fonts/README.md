# Deterministic sparse text test fonts

These files are unchanged copies of Web Platform Tests fixtures, obtained from
the local Genet checkout. They are test data under their own SIL Open Font
License 1.1 notices, not MPL-licensed project source. No system fonts or font
downloads are required when the tests run.

Source directory: `tests/wpt/tests/css/css-fonts/variations/resources/` in
Genet, last source-file commit `1359e8e4624d10ceb0bf55bcb1cb2903141c6d38`.
Upstream directory:
https://github.com/web-platform-tests/wpt/tree/master/css/css-fonts/variations/resources

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `Inter.var.subset.ttf` | 5080 | `e4c84cd1770b9f44e8b7b73c0d2607170de497d2b5d1c32d248a874b40131f30` |
| `variabletest_box.ttf` | 4032 | `9270b7f6b2b8b34215a80c63a9c1348e704a0e3803ee49f2531bd62fdb5139c4` |

Inter embeds Copyright 2020 The Inter Project Authors. Its complete OFL notice
is copied from https://raw.githubusercontent.com/rsms/inter/v3.19/LICENSE.txt
into `Inter-LICENSE.txt`. The subset contains lowercase `a`, `l`, `n`, `s`,
and `t`, with `wght` and `slnt` variation axes.

`variabletest_box.ttf` embeds Copyright 2017 The Chromium Authors and an explicit
SIL Open Font License 1.1 declaration. `variabletest_box-LICENSE.txt` preserves
that copyright and the full standard OFL 1.1 text. The OFL text was copied from
the Vello fork's `examples/assets/inconsolata/LICENSE.txt` at
`ca3f40ea182216883cd543c7b9deae991268917c`; it is standard license text, not a
claim that the box font derives from Inconsolata.

The WPT `css/css-fonts/variations/variable-box-font.html` fixture documents the
independent variation oracle: `A` at its default `UPWD` value has the shape of
LOWER HALF BLOCK; `UPWD=350` has the shape of UPPER HALF BLOCK. Inter lacks `A`
and the box font supplies it, allowing caller-selected two-font fallback to be
tested without inventing font fallback behavior inside NetRender.

The test also builds a TTC v1 container in memory with Inter as face 0 and the
box font as face 1. It rebases each SFNT table offset to the collection origin
and zeros the unused per-face `checkSumAdjustment`. Outline tables are unchanged;
there is no additional generated font artifact to retain. The fixture is parsed
independently with Skrifa before checking nonzero collection-index rendering.
