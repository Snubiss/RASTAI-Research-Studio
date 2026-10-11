# Command golden: copy as

`copy-as.json` writes the same command call four ways, as the palette's "Copy as" menu does (layout `frontend-and-plugins.md` §2.5 step 7; control-plane layout §10.3). Every surface reads this one file, so the UI's formatter, the CLI's parser and the naming rules can't drift apart.

Each entry of `examples` holds:

| Field | What it holds |
|---|---|
| `command_id` | the command (contracts §6.4) |
| `arguments` | the call's arguments as JSON |
| `cli` | `rastai <words> --<flag> <value> ...`: the words are `cli_words` of the id and the flags are `flag_name` of each argument field, in the order of `arguments`; flags only, never positionals |
| `python` | `rastai.connect().execute("<command_id>", {...})` |
| `javascript` | `await client.execute("<command_id>", {...})` |
| `mcp` | the `tools/call` parameters: `name` is `mcp_tool_name` of the id, `arguments` the call's arguments |

This is the seed (M1-21): commands chosen for their naming cases (an underscore in the noun: `login_code`, `crash_report`; in a flag: `max_new_tokens`; a plugin command: `eval.test`), with argument fields the contracts already name (§6.4–6.5 and the frontend layout's example). The CLI lines use only text and whole-number values; the CLI's syntax for true/false flags, lists and quoting belongs to `rastai-cli` (M1-31), which adds those cases here.

**Who reads it.**

| Test | What it checks |
|---|---|
| `crates/rastai-core/tests/naming_round_trip.rs` | the CLI words and flags and the MCP name of every entry follow the naming rules, and the CLI words lead back to the command id |
| `apps/web/src/commands/copy-as.test.js` (M1-54) | the formatter writes every line of every entry |
| `crates/rastai-cli/tests/verbs.rs` (M1-31) | every CLI line parses back to its command id and arguments |
