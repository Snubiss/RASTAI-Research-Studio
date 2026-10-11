# Diagnostic render snapshots

Inputs and expected output for `crates/rastai-diag/tests/render_snapshots.rs`: one case per severity and per shape of
`Diagnostic.path` (contracts 4.1).

- `<case>.json`: the diagnostic, the catalog `title` and `basic_text` to render it with, and optionally
  `source_file`, the text the path points into. The texts are written for these tests; they are not the catalog's.
- `<case>.txt`: `render_text` output in `Plain` style. The `Color` output must equal it once the ANSI codes are removed.
- `<case>.basic.txt`: `render_basic` output.
- `textcnn_source.txt`: the source text the located cases point into.
