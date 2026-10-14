//! Flags to arguments JSON, from the command's argument schema (layout: rust-control-plane.md 9.4). The schema is
//! the descriptor's `arguments_schema_json` (`json_schema_for` in rastai-formats), so a new command or a plugin's
//! gets its flags with no code here.
//!
//! | Field (from the schema) | Command line |
//! |---|---|
//! | string, integer, number | `--name value` or `--name=value` |
//! | boolean | `--name` / `--no-name` (`--name=false` too) |
//! | enum | `--name value`, checked against the list |
//! | list | the flag repeated |
//! | map | `--name key=value`, repeated |
//! | object, any JSON, a type | only through `--arguments-json` or `--arguments-file` |
//! | `x-rastai-cli-position` | positional, in order, or as its flag |
//!
//! Flags override `--arguments-json` and `--arguments-file` key by key. The result is checked here for a fast
//! message (unknown flags, missing required fields, enum values); the daemon checks it again in full.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use rastai_core::commands::naming::{cli_words, flag_name, negated_flag_name};
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_formats::error::FormatsError;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::syntax::syntax_diagnostic;
use rastai_proto::commands::v1::CommandDescriptor;
use serde_json::{Map, Value, json};

use crate::arguments::{CommandLineItem, ParsedArguments, leading_words, shown_flag};

const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
/// The schema keyword of a positional field (contracts 5.2, `cli_position` in rastai-formats).
const CLI_POSITION_KEYWORD: &str = "x-rastai-cli-position";
const MAP_SEPARATOR: char = '=';
const TRUE_WORD: &str = "true";
const FALSE_WORD: &str = "false";

// What the refusals say was expected.
const EXPECTED_OBJECT: &str = "a JSON object";
const EXPECTED_ONE_SOURCE: &str = "--arguments-json or --arguments-file, not both";
const EXPECTED_NO_MORE_WORDS: &str = "no more positional words";
const EXPECTED_NO_FLAGS: &str = "no flags: this command takes none";
const EXPECTED_JSON_ONLY: &str = "this field only through --arguments-json or --arguments-file";
const EXPECTED_MAP_ENTRY: &str = "key=value";
const EXPECTED_NO_VALUE: &str = "no value after an off flag";
/// How much of a refused `--arguments-json` text a refusal shows: enough to recognise it, never a whole file's worth.
const SHOWN_TEXT_CHARACTERS: usize = 200;
const CUT_MARK: &str = "...";

/// What a field takes on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagKind {
    Text,
    Integer,
    Number,
    Boolean,
    /// An enum: one of these words.
    Choice(Vec<String>),
    /// The flag repeated, one element each.
    List(Box<FlagKind>),
    /// The flag repeated, one `key=value` each.
    Map(Box<FlagKind>),
    /// An object, any JSON value or a named type: no flag, only `--arguments-json` or `--arguments-file`.
    JsonOnly,
}

/// One property of the argument schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentField {
    /// The field name, `max_new_tokens`.
    pub name: String,
    /// `--max-new-tokens`.
    pub flag: String,
    pub kind: FlagKind,
    pub required: bool,
    pub cli_position: Option<u64>,
    pub description: String,
}

/// The fields of an argument schema, in the schema's order. A schema that isn't an object has none.
pub fn argument_fields(schema: &Value) -> Vec<ArgumentField> {
    let required: Vec<&str> = match schema["required"].as_array() {
        Some(names) => names.iter().filter_map(Value::as_str).collect(),
        None => Vec::new(),
    };
    let Some(properties) = schema["properties"].as_object() else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    for (name, property) in properties {
        fields.push(ArgumentField {
            name: name.clone(),
            flag: flag_name(name),
            kind: flag_kind(property),
            required: required.contains(&name.as_str()),
            cli_position: property[CLI_POSITION_KEYWORD].as_u64(),
            description: property["description"].as_str().unwrap_or_default().to_string(),
        });
    }
    fields
}

/// What the schema of one property allows on the command line (the schema shapes of rastai-formats' json_schema.rs).
fn flag_kind(property: &Value) -> FlagKind {
    if let Some(values) = property["enum"].as_array() {
        return FlagKind::Choice(values.iter().filter_map(Value::as_str).map(str::to_string).collect());
    }
    match property["type"].as_str() {
        Some("string") => FlagKind::Text,
        Some("integer") => FlagKind::Integer,
        Some("number") => FlagKind::Number,
        Some("boolean") => FlagKind::Boolean,
        Some("array") => element_kind(&property["items"], FlagKind::List),
        // A map's members have a schema of their own; any other object is a typed message.
        Some("object") if property["additionalProperties"].is_object() => {
            element_kind(&property["additionalProperties"], FlagKind::Map)
        }
        _ => FlagKind::JsonOnly,
    }
}

/// A list or a map takes flags only when its elements are plain values.
fn element_kind(element: &Value, wrap: fn(Box<FlagKind>) -> FlagKind) -> FlagKind {
    match flag_kind(element) {
        FlagKind::List(_) | FlagKind::Map(_) | FlagKind::JsonOnly => FlagKind::JsonOnly,
        plain => wrap(Box::new(plain)),
    }
}

/// The arguments JSON of one invocation of `command`: `--arguments-json` or `--arguments-file` first, then the
/// flags and positional words after the verb. Every problem found is returned, each `RRS-C0002`.
pub fn build_arguments_json(
    command: &CommandDescriptor,
    schema: &Value,
    arguments: &ParsedArguments,
) -> Result<String, Vec<Diagnostic>> {
    let fields = argument_fields(schema);
    let mut values = base_arguments(arguments).map_err(|refusal| vec![refusal])?;
    let verb_words = verb_word_count(command, arguments);
    let mut reader = CommandLineReader::default();
    reader.read_items(&fields, &arguments.items[verb_words..]);
    reader
        .positional_words
        .extend(arguments.words_after_flags.iter().cloned());
    reader.place_positionals(&fields);
    reader.apply_to(&mut values);
    reader.problems.extend(check_fields(&fields, &values));
    if !reader.problems.is_empty() {
        return Err(reader.problems);
    }
    Ok(Value::Object(values).to_string())
}

/// How many leading words named the command: its CLI words, or one for an alias (`doctor`).
fn verb_word_count(command: &CommandDescriptor, arguments: &ParsedArguments) -> usize {
    let words = cli_words(&command.command_id);
    let leading = leading_words(arguments);
    if leading.len() >= words.len() && leading[..words.len()] == words[..] {
        return words.len();
    }
    1
}

/// `--arguments-json` or `--arguments-file`, which must hold a JSON object; none is the empty object. A refusal
/// names the file, or shows the start of the text: never a whole file's worth.
// A refusal is a `Diagnostic`, the contract type every caller renders; refusals are rare, so it isn't boxed.
#[allow(clippy::result_large_err)]
fn base_arguments(arguments: &ParsedArguments) -> Result<Map<String, Value>, Diagnostic> {
    let global = &arguments.global;
    let (text, shown) = match (&global.arguments_json, &global.arguments_file) {
        (Some(_), Some(_)) => return Err(invalid("--arguments-json and --arguments-file", EXPECTED_ONE_SOURCE)),
        (Some(text), None) => (text.clone(), shortened(text)),
        (None, Some(path)) => {
            let text = read_arguments_file(path, ImportLimits::default().max_file_bytes)?;
            (text, path.clone())
        }
        (None, None) => return Ok(Map::new()),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err(invalid(&shown, EXPECTED_OBJECT)),
    }
}

/// The text of an `--arguments-file`, read at most one byte past `max_bytes`, the same cap `validate` puts on a file
/// (local_commands.rs). A FIFO or a growing file can't fill memory, and a file past the cap is refused with the
/// size code (`RRS-E5003`) before any JSON is parsed. Text that isn't UTF-8 is refused like any other non-object.
#[allow(clippy::result_large_err)]
fn read_arguments_file(path: &str, max_bytes: u64) -> Result<String, Diagnostic> {
    let opened = File::open(Path::new(path)).map_err(|_| invalid(path, EXPECTED_OBJECT))?;
    // A FIFO or a device says 0 here, so the size it reports is only the best guess once the read says more.
    let size_said = opened.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let mut bytes = Vec::new();
    opened
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| invalid(path, EXPECTED_OBJECT))?;
    let size_read = bytes.len() as u64;
    if size_read > max_bytes {
        return Err(too_large(path, size_said.max(size_read), max_bytes));
    }
    String::from_utf8(bytes).map_err(|_| invalid(path, EXPECTED_OBJECT))
}

/// The size refusal `validate` gives a file past the import limit, so both commands name the same code.
fn too_large(path: &str, size_bytes: u64, limit_bytes: u64) -> Diagnostic {
    let error = FormatsError::FileTooLarge {
        size_bytes,
        limit_bytes,
    };
    syntax_diagnostic(&error, path)
}

/// The first `SHOWN_TEXT_CHARACTERS` characters of a text, marked when cut.
fn shortened(text: &str) -> String {
    if text.chars().count() <= SHOWN_TEXT_CHARACTERS {
        return text.to_string();
    }
    let start: String = text.chars().take(SHOWN_TEXT_CHARACTERS).collect();
    format!("{start}{CUT_MARK}")
}

/// What the words after the verb set, collected before any of it is applied, so a repeated list flag gathers all
/// its elements and later flags win over earlier ones.
#[derive(Default)]
struct CommandLineReader {
    set_values: BTreeMap<String, Value>,
    list_values: BTreeMap<String, Vec<Value>>,
    map_values: BTreeMap<String, Map<String, Value>>,
    positional_words: Vec<String>,
    problems: Vec<Diagnostic>,
}

impl CommandLineReader {
    fn read_items(&mut self, fields: &[ArgumentField], items: &[CommandLineItem]) {
        let mut index = 0;
        while index < items.len() {
            match &items[index] {
                CommandLineItem::Word(word) => self.positional_words.push(word.clone()),
                CommandLineItem::Flag { name, inline_value } => {
                    let next_word = word_at(items, index + 1);
                    if self.read_flag(fields, name, inline_value, next_word) {
                        index += 1;
                    }
                }
            }
            index += 1;
        }
    }

    /// Reads one flag; true when it took the next word as its value.
    fn read_flag(
        &mut self,
        fields: &[ArgumentField],
        name: &str,
        inline_value: &Option<String>,
        next_word: Option<&str>,
    ) -> bool {
        let shown = shown_flag(name, inline_value);
        let Some((field, negated)) = find_flag(fields, name) else {
            self.problems.push(invalid(&shown, &flag_list(fields)));
            return false;
        };
        if field.kind == FlagKind::Boolean {
            self.read_switch(field, negated, inline_value, &shown);
            return false;
        }
        let (text, took_next_word) = match (inline_value, next_word) {
            (Some(value), _) => (value.clone(), false),
            (None, Some(word)) => (word.to_string(), true),
            (None, None) => {
                self.problems.push(invalid(&shown, &expected_value(field)));
                return false;
            }
        };
        self.read_value(field, &text, &shown);
        took_next_word
    }

    fn read_switch(&mut self, field: &ArgumentField, negated: bool, inline_value: &Option<String>, shown: &str) {
        let value = match (negated, inline_value.as_deref()) {
            (true, None) => false,
            (true, Some(_)) => {
                self.problems.push(invalid(shown, EXPECTED_NO_VALUE));
                return;
            }
            (false, None) | (false, Some(TRUE_WORD)) => true,
            (false, Some(FALSE_WORD)) => false,
            (false, Some(_)) => {
                self.problems.push(invalid(shown, &expected_value(field)));
                return;
            }
        };
        self.set_values.insert(field.name.clone(), Value::Bool(value));
    }

    fn read_value(&mut self, field: &ArgumentField, text: &str, shown: &str) {
        let parsed = match &field.kind {
            FlagKind::List(element) => self.read_list_element(field, element, text),
            FlagKind::Map(member) => self.read_map_entry(field, member, text),
            FlagKind::JsonOnly => Err(EXPECTED_JSON_ONLY.to_string()),
            plain => self.read_plain_value(field, plain, text),
        };
        if let Err(expected) = parsed {
            self.problems.push(invalid(&format!("{shown} {text}"), &expected));
        }
    }

    fn read_list_element(&mut self, field: &ArgumentField, element: &FlagKind, text: &str) -> Result<(), String> {
        match parse_plain(element, text) {
            Ok(value) => {
                self.list_values.entry(field.name.clone()).or_default().push(value);
                Ok(())
            }
            Err(expected) => Err(expected),
        }
    }

    fn read_plain_value(&mut self, field: &ArgumentField, kind: &FlagKind, text: &str) -> Result<(), String> {
        match parse_plain(kind, text) {
            Ok(value) => {
                self.set_values.insert(field.name.clone(), value);
                Ok(())
            }
            Err(expected) => Err(expected),
        }
    }

    fn read_map_entry(&mut self, field: &ArgumentField, member: &FlagKind, text: &str) -> Result<(), String> {
        let Some((key, member_text)) = text.split_once(MAP_SEPARATOR) else {
            return Err(EXPECTED_MAP_ENTRY.to_string());
        };
        if key.is_empty() {
            return Err(EXPECTED_MAP_ENTRY.to_string());
        }
        let value = parse_plain(member, member_text)?;
        self.map_values
            .entry(field.name.clone())
            .or_default()
            .insert(key.to_string(), value);
        Ok(())
    }

    /// Positional words fill the fields with `x-rastai-cli-position` that no flag set, in position order. A list
    /// field takes every word left.
    fn place_positionals(&mut self, fields: &[ArgumentField]) {
        let mut positional: Vec<&ArgumentField> = fields.iter().filter(|field| field.cli_position.is_some()).collect();
        positional.sort_by_key(|field| field.cli_position);
        let mut words = std::mem::take(&mut self.positional_words).into_iter();
        for field in positional {
            if self.set_values.contains_key(&field.name) || self.list_values.contains_key(&field.name) {
                continue;
            }
            if matches!(field.kind, FlagKind::List(_)) {
                for word in words.by_ref() {
                    self.read_value(field, &word, &field.flag);
                }
                continue;
            }
            if let Some(word) = words.next() {
                self.read_value(field, &word, &field.flag);
            }
        }
        for word in words {
            self.problems.push(invalid(&word, EXPECTED_NO_MORE_WORDS));
        }
    }

    /// Writes what the command line set over the base arguments, key by key.
    fn apply_to(&mut self, values: &mut Map<String, Value>) {
        for (name, value) in std::mem::take(&mut self.set_values) {
            values.insert(name, value);
        }
        for (name, elements) in std::mem::take(&mut self.list_values) {
            values.insert(name, Value::Array(elements));
        }
        for (name, members) in std::mem::take(&mut self.map_values) {
            values.insert(name, Value::Object(members));
        }
    }
}

/// The field a flag names, and whether it was its `--no-` form (booleans only).
fn find_flag<'a>(fields: &'a [ArgumentField], flag: &str) -> Option<(&'a ArgumentField, bool)> {
    for field in fields {
        if field.flag == flag {
            return Some((field, false));
        }
        if field.kind == FlagKind::Boolean && negated_flag_name(&field.name) == flag {
            return Some((field, true));
        }
    }
    None
}

/// Required fields present, and keys the schema doesn't have refused (it allows no others).
fn check_fields(fields: &[ArgumentField], values: &Map<String, Value>) -> Vec<Diagnostic> {
    let mut problems = Vec::new();
    for field in fields {
        if field.required && !values.contains_key(&field.name) {
            problems.push(invalid(&format!("no {}", field.flag), &expected_value(field)));
        }
    }
    for key in values.keys() {
        if !fields.iter().any(|field| &field.name == key) {
            problems.push(invalid(key, &flag_list(fields)));
        }
    }
    problems
}

/// A plain value from its text, or what was expected instead.
fn parse_plain(kind: &FlagKind, text: &str) -> Result<Value, String> {
    let parsed = match kind {
        FlagKind::Text => Some(json!(text)),
        FlagKind::Integer => parse_integer(text),
        FlagKind::Number => text
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(|number| json!(number)),
        FlagKind::Boolean => match text {
            TRUE_WORD => Some(json!(true)),
            FALSE_WORD => Some(json!(false)),
            _ => None,
        },
        FlagKind::Choice(choices) => choices.iter().any(|choice| choice == text).then(|| json!(text)),
        FlagKind::List(_) | FlagKind::Map(_) | FlagKind::JsonOnly => None,
    };
    parsed.ok_or_else(|| kind_words(kind))
}

/// A whole number: negative ones as i64, the largest positive ones as u64.
fn parse_integer(text: &str) -> Option<Value> {
    if let Ok(number) = text.parse::<i64>() {
        return Some(json!(number));
    }
    text.parse::<u64>().ok().map(|number| json!(number))
}

/// How a refusal describes what a kind of field takes.
pub fn kind_words(kind: &FlagKind) -> String {
    match kind {
        FlagKind::Text => "text".to_string(),
        FlagKind::Integer => "a whole number".to_string(),
        FlagKind::Number => "a number".to_string(),
        FlagKind::Boolean => "true or false".to_string(),
        FlagKind::Choice(choices) => format!("one of {}", choices.join(", ")),
        FlagKind::List(element) => format!("{}, the flag repeated", kind_words(element)),
        FlagKind::Map(member) => format!("key=value with {} values, the flag repeated", kind_words(member)),
        FlagKind::JsonOnly => EXPECTED_JSON_ONLY.to_string(),
    }
}

fn expected_value(field: &ArgumentField) -> String {
    format!("{} {}", field.flag, kind_words(&field.kind))
}

/// The command's flags, for a refusal of one it doesn't have.
fn flag_list(fields: &[ArgumentField]) -> String {
    if fields.is_empty() {
        return EXPECTED_NO_FLAGS.to_string();
    }
    let flags: Vec<&str> = fields.iter().map(|field| field.flag.as_str()).collect();
    flags.join(", ")
}

fn word_at(items: &[CommandLineItem], index: usize) -> Option<&str> {
    match items.get(index) {
        Some(CommandLineItem::Word(word)) => Some(word),
        _ => None,
    }
}

fn invalid(found: &str, expected: &str) -> Diagnostic {
    diagnostic(CODE_INVALID_ARGUMENTS, &[("found", found), ("expected", expected)])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn schema_kinds_become_flag_kinds() {
        let schema = json!({"properties": {
            "a": {"type": "string"},
            "b": {"type": "string", "enum": ["x", "y"]},
            "c": {"type": "array", "items": {"type": "integer"}},
            "d": {"type": "object", "additionalProperties": {"type": "string"}},
            "e": {"$ref": "#/$defs/T"},
            "f": {},
            "g": {"type": "object", "description": "Protobuf JSON of rastai.common.v1.X."},
            "h": {"type": "array", "items": {"$ref": "#/$defs/T"}}
        }});
        let kinds: Vec<FlagKind> = argument_fields(&schema).into_iter().map(|field| field.kind).collect();
        assert_eq!(
            kinds,
            [
                FlagKind::Text,
                FlagKind::Choice(vec!["x".to_string(), "y".to_string()]),
                FlagKind::List(Box::new(FlagKind::Integer)),
                FlagKind::Map(Box::new(FlagKind::Text)),
                FlagKind::JsonOnly,
                FlagKind::JsonOnly,
                FlagKind::JsonOnly,
                FlagKind::JsonOnly,
            ]
        );
    }

    #[test]
    fn a_command_with_no_fields_takes_no_flags() {
        assert_eq!(flag_list(&[]), "no flags: this command takes none");
    }

    fn with_arguments(arguments_json: Option<String>, arguments_file: Option<String>) -> ParsedArguments {
        let mut parsed = ParsedArguments::default();
        parsed.global.arguments_json = arguments_json;
        parsed.global.arguments_file = arguments_file;
        parsed
    }

    #[test]
    fn arguments_that_are_not_an_object_are_shown_short() {
        let long_list = format!("[{}0]", "0,".repeat(5_000));
        let refusal = base_arguments(&with_arguments(Some(long_list.clone()), None)).unwrap_err();
        assert_eq!(refusal.code, "RRS-C0002");
        assert!(refusal.message.contains("[0,0,0,"), "{}", refusal.message);
        assert!(refusal.message.contains("..."), "{}", refusal.message);
        assert!(refusal.message.len() < long_list.len() / 10, "{}", refusal.message);

        let file = std::env::temp_dir().join(format!("rastai-cli-arguments-{}.json", std::process::id()));
        std::fs::write(&file, &long_list).unwrap();
        let path = file.display().to_string();
        let refusal = base_arguments(&with_arguments(None, Some(path.clone()))).unwrap_err();
        let _ = std::fs::remove_file(&file);
        assert!(refusal.message.contains(&path), "{}", refusal.message);
        assert!(!refusal.message.contains("[0,0,0,"), "{}", refusal.message);
    }

    fn scratch_path(name: &str) -> String {
        std::env::temp_dir()
            .join(format!("rastai-cli-{name}-{}.json", std::process::id()))
            .display()
            .to_string()
    }

    #[test]
    fn an_arguments_file_is_read_up_to_its_limit_and_no_further() {
        let path = scratch_path("at-limit");
        std::fs::write(&path, "{}    ").unwrap();
        let read = read_arguments_file(&path, 6);
        std::fs::write(&path, "{}     ").unwrap();
        let too_long = read_arguments_file(&path, 6);
        let _ = std::fs::remove_file(&path);
        assert_eq!(read.unwrap(), "{}    ");
        let refusal = too_long.unwrap_err();
        assert_eq!(refusal.code, "RRS-E5003");
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_past_the_limit_is_refused_with_the_size_code() {
        let fifo = scratch_path("fifo");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
        assert!(made.success());
        let limit = ImportLimits::default().max_file_bytes;
        let padding = limit as usize + 1024;
        let writer_path = fifo.clone();
        // The reader stops after limit+1 bytes and closes the pipe, so the writer's last writes fail; that is the
        // expected end of this stream, not a test failure.
        let writer = std::thread::spawn(move || {
            let mut pipe = std::fs::OpenOptions::new().write(true).open(&writer_path).unwrap();
            let _ = std::io::Write::write_all(&mut pipe, &vec![b'{'; padding]);
        });
        let refusal = base_arguments(&with_arguments(None, Some(fifo.clone()))).unwrap_err();
        writer.join().unwrap();
        let _ = std::fs::remove_file(&fifo);
        assert_eq!(refusal.code, "RRS-E5003", "{}", refusal.message);
    }

    #[test]
    fn integers_keep_their_sign_and_size() {
        assert_eq!(parse_integer("-3"), Some(json!(-3)));
        assert_eq!(parse_integer("18446744073709551615"), Some(json!(u64::MAX)));
        assert_eq!(parse_integer("1.5"), None);
    }
}
