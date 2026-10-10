# ADR-0004: Protobuf owns the wire, rastai-formats owns documents, everything else is generated

**Status:** Accepted, 2026-10-10

## Context

- Three languages (Rust, Python, TypeScript) share wire messages, document types and command arguments.
- Hand-copied types drift (§28 risk 8). Drift shows up as silent field loss, not as compile errors.
- §20 names pydantic for the generated Python types, but pydantic isn't on the §27 allowlist. The owner's code style rules out metaclass-driven models (ADR-0023).
- Documents are user-facing JSON with a `$schema` key. Wire messages are machine-only.

## Decision

1. **Wire types:** protobuf in `proto/rastai/{common,commands,events,node,worker,provider}/v1/`, linted and generated with buf:
   - Rust: `rastai-proto` (prost + tonic);
   - Python: `rastai_proto` (grpcio);
   - TS: `@rastai/proto` (protobuf-es + connect-es).
   Generated code is never edited by hand.
2. **Document and command-argument types:** plain Rust structs with serde derives in `rastai-formats`. Next to each struct sits one hand-written `describe_<type>() -> TypeDescription` (plain data: field name, kind, required, doc string). A test fills a fixture and checks that the struct and its description list the same fields.
3. **Codegen:** the `rastai-formats-codegen` binary walks the descriptions and writes:
   - `schemas/<type>/<schema_version>.json`;
   - `packages/formats/src/generated/` (TS types);
   - `python/rastai_formats/generated/` (plain `@dataclass` classes, not pydantic).
4. **Validation happens once, in Rust.** Python gets it through the PyO3 wheel (`rastai_formats`) and the browser through the WASM build (`@rastai/formats`).
5. **Documents cross the wire as JSON strings** (`string <name>_json`), validated by `rastai-formats` at both ends. Protobuf never re-declares a document's fields.
6. **Proto rules:**
   - times are `int64 <name>_unix_ms` and sizes `uint64 <name>_bytes`;
   - no `google.protobuf.Timestamp`;
   - field numbers are never reused;
   - every failure carries a `rastai.common.v1.Diagnostic`.
7. **CI:**
   - `buf breaking` on every PR;
   - golden messages in `fixtures/proto-golden/`, round-tripped in Rust, TS and Python;
   - `tools/checks/check-generated` fails if regenerating changes any file;
   - schema snapshot checks.
8. **Contract changes** (proto, schemas, command catalog) land in their own PR, before any code that uses them.

```rust
// crates/rastai-formats/src/documents/eval.rs (shortened; the shape of control-plane layout §4.2)
const EVAL_SCHEMA_ID: &str = "rastai.eval/1";

#[derive(Serialize, Deserialize, Clone)]
pub struct EvalSuite { pub header: SharedHeader, pub tests: Vec<EvalTest>, pub pass_rule: PassRule }

pub fn describe_eval_suite() -> TypeDescription {
    TypeDescription {
        name: "EvalSuite".into(),
        schema_id: EVAL_SCHEMA_ID.into(),
        doc: "Model tests that pass or fail as a whole.".into(),
        fields: vec![
            field("tests", list_of(FieldKind::Type("EvalTest".into())), REQUIRED, "Model tests in this suite."),
            field("pass_rule", FieldKind::Type("PassRule".into()), REQUIRED, "How test results combine into pass or fail."),
        ],
    }
}
```

The example follows the control-plane layout (§4.1–4.2) and contracts §5.4: `TypeDescription { name, schema_id, doc, fields }`, `FieldKind::List(Box<FieldKind>)` built with `list_of()`, `FieldKind::Type(String)` for another described type, and `FieldKind::Proto` for a wire type stored inside a document. *Note since acceptance (2026-10-10): only this illustration was updated to the layout's names; the decision is unchanged.*

## Consequences

**Good**
- Each type has exactly one definition. A field added in Rust shows up in TS and Python on the next codegen, and CI catches anyone who forgot.
- No schema library and no proc macros of our own. The description table is plain data anyone can read.
- Python stays light (dataclasses), and validation can't disagree between languages because there is only one implementation.

**Costs**
- The `describe_<type>()` tables are hand-written next to the structs. The field-match test stops them drifting, but they're extra lines. If this proves tedious, allowlisting `schemars` is the fallback (contracts §14 item 4).
- Document payloads in proto are opaque strings, so gRPC tooling can't show their fields.
- Codegen is one more build step. `just` runs it, and CI checks its output.

## Alternatives considered

| Alternative | Why not |
|---|---|
| JSON Schema as the source, Rust generated from it | Loses hand-written Rust types with real enums. Generated Rust is hard to read. |
| Protobuf for documents too | Users edit and diff these files. Protobuf's JSON mapping is awkward and has no `$schema` story. |
| pydantic models | Not on the allowlist, and it would duplicate validation outside Rust. |
| `schemars` derive | Allowed later if the tables prove tedious. For now, no external crate is needed. |
| Hand-written TS and Python types | They drift. This is the failure the rule exists to prevent. |

## Links

- Spec: [§20](../spec/rastai_spec_v5.md#20-schemas-protocols-and-versioning), [§23](../spec/rastai_spec_v5.md#23-validation), [§29](../spec/rastai_spec_v5.md#29-repo-and-engineering-setup)
- Contracts: [§4 wire](../architecture/contracts.md#4-wire-contracts), [§5 documents](../architecture/contracts.md#5-documents-owned-by-rastai-formats), [§14](../architecture/contracts.md#14-open-questions) items 3 and 4
- Related: [ADR-0005](0005-command-layer.md), [ADR-0023](0023-owner-code-style.md), [ADR-0032](0032-document-files-and-validation.md)
