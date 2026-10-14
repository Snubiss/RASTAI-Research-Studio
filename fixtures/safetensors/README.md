# Safetensors fixtures

The strict-contract corpus of spike 07: 33 hand-built safetensors files, 6 valid and 27 malformed (contracts §12.5; ADR-0029 decision 5, ADR-0037 decision 3). `expected.json` gives each file's verdict: `{"accepted": true, "tensors": <count>}` for a valid file, `{"accepted": false, "code": "RRS-E60xx"}` for a refused one, one code per kind of problem (contracts §10.3).

**Where they come from.** `tools/fixtures/make_safetensors_corpus.py` writes every file and `expected.json` from fixed values; the bytes are the spike's (`spike/src/rastai_spike/bakeoff_fixtures.py`), and `tools/fixtures/tests/test_make_safetensors_corpus.py` checks that the generator writes these exact files. The spike kept each verdict as its Python reader's error text; here each is the catalog code the one header check in `rastai-formats` must give. One verdict changed on the way: `shape_overflows_64_bits` is an overflow (`RRS-E6012`), which the spike's Python reader, with its arbitrary-precision integers, called a size mismatch.

**Who reads them.**

| Test | What it checks |
|---|---|
| `crates/rastai-formats/tests/safetensors_corpus.rs` | every file gives its verdict, tensor count and code from `read_safetensors_header_length` and `check_safetensors_header` in Rust |
| `python/rastai_formats/tests/test_parity.py` | the same verdicts and codes through the wheel |
| `python/rastai_models/tests/test_safetensors_reader.py` | the Python reader gives the same verdicts and codes, reads the valid files' tensors and closes every file it refused |
| `python/rastai_models/tests/test_safetensors_delegation.py` | with the wheel's checks replaced by ones that refuse everything, the reader refuses all 33 files, the 6 valid ones too, and reads no tensor bytes |

**The writer's layout.** `fixtures/safetensors-writer/planned_layout.safetensors` (a sibling folder, so the corpus check above stays exact) is the file `write_safetensors` writes for three fixed tensors and two metadata keys. `python/rastai_models/tests/test_safetensors_writer.py` checks the writer still writes these bytes, and `crates/rastai-formats/src/safetensors/writer.rs` reads the file and checks its header and offsets against `plan_safetensors`. It is not in the corpus: it is valid, and has no verdict in `expected.json`.

**Changing them.** The files are frozen test inputs. A new edge case is a new case in the generator, with its expected code, and a new file here; never an edited file.
