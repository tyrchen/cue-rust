//! Encoding adapters for external data formats.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{borrow::Cow, str, str::FromStr};

use bigdecimal::BigDecimal;
use cue_rust_eval::{
    EvalError, EvaluatedValue, ExportOptions, NumericBound, StringConstraint, StringConstraintSet,
    ValidateOptions, Value,
};
use cue_rust_source::SourceLimits;
use noyalib::{Mapping as YamlMapping, Number as YamlNumber, Value as YamlValue};
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use thiserror::Error;
use toml::{Table as TomlTable, Value as TomlValue};

const DEFAULT_MAX_DECODE_DEPTH: u32 = 128;
const DEFAULT_MAX_COLLECTION_ITEMS: usize = 65_536;
const DEFAULT_MAX_STRING_BYTES: usize = 1_048_576;

/// Supported external data encodings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Encoding {
    /// CUE-like value syntax.
    Cue,
    /// JavaScript Object Notation.
    Json,
    /// YAML data streams.
    Yaml,
    /// TOML documents.
    Toml,
}

/// Options shared by data decoders.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DecodeOptions {
    /// Source byte limits.
    pub source_limits: SourceLimits,
    /// Maximum decoded nesting depth.
    pub max_depth: u32,
    /// Maximum items in any decoded collection.
    pub max_collection_items: usize,
    /// Maximum bytes in any decoded string.
    pub max_string_bytes: usize,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            source_limits: SourceLimits::default(),
            max_depth: DEFAULT_MAX_DECODE_DEPTH,
            max_collection_items: DEFAULT_MAX_COLLECTION_ITEMS,
            max_string_bytes: DEFAULT_MAX_STRING_BYTES,
        }
    }
}

/// Options shared by data encoders.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EncodeOptions {
    /// Output encoding.
    pub encoding: Encoding,
    /// Require concrete values before encoding.
    pub concrete: bool,
    /// Field visibility options for export.
    pub export_options: ExportOptions,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            encoding: Encoding::Json,
            concrete: true,
            export_options: ExportOptions::default(),
        }
    }
}

/// Errors produced while decoding external data.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// Input exceeded the configured byte limit.
    #[error("input is too large: {actual} bytes exceeds limit {limit} bytes")]
    SourceTooLarge {
        /// Observed byte length.
        actual: usize,
        /// Configured limit.
        limit: usize,
    },
    /// Input bytes were not valid UTF-8.
    #[error(transparent)]
    Utf8(#[from] str::Utf8Error),
    /// JSON parsing failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// YAML parsing failed.
    #[error("YAML decode failed: {0}")]
    Yaml(String),
    /// TOML parsing failed.
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    /// The decoded data uses a value unsupported by the CUE core subset.
    #[error("unsupported decoded value: {0}")]
    Unsupported(String),
    /// Decoded nesting exceeded configured limits.
    #[error("decoded value exceeds maximum depth {limit}")]
    MaxDepth {
        /// Configured depth limit.
        limit: u32,
    },
    /// Decoded collection exceeded configured limits.
    #[error("decoded collection has {actual} items, exceeding limit {limit}")]
    MaxCollectionItems {
        /// Observed item count.
        actual: usize,
        /// Configured item limit.
        limit: usize,
    },
    /// Decoded string exceeded configured limits.
    #[error("decoded string has {actual} bytes, exceeding limit {limit}")]
    MaxStringBytes {
        /// Observed byte count.
        actual: usize,
        /// Configured byte limit.
        limit: usize,
    },
}

/// Errors produced while encoding CUE values.
#[derive(Debug, Error)]
pub enum EncodeError {
    /// Evaluation failed before encoding.
    #[error(transparent)]
    Eval(#[from] EvalError),
    /// JSON encoding failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// YAML encoding failed.
    #[error("YAML encode failed: {0}")]
    Yaml(String),
    /// TOML encoding failed.
    #[error(transparent)]
    Toml(#[from] toml::ser::Error),
    /// The value cannot be represented in the requested encoding.
    #[error("unsupported value for {encoding:?}: {message}")]
    Unsupported {
        /// Requested encoding.
        encoding: Encoding,
        /// Human-readable message.
        message: String,
    },
}

/// Decodes bytes in an external data format into a value.
///
/// # Errors
///
/// Returns [`DecodeError`] when parsing fails or the input exceeds limits.
pub fn decode_bytes(
    encoding: Encoding,
    bytes: &[u8],
    options: DecodeOptions,
) -> Result<Value, DecodeError> {
    validate_size(bytes, options.source_limits)?;
    let evaluated = match encoding {
        Encoding::Json => json_to_evaluated(serde_json::from_slice(bytes)?, options, 0)?,
        Encoding::Yaml => {
            let value: JsonValue =
                noyalib::from_slice(bytes).map_err(|error| DecodeError::Yaml(error.to_string()))?;
            json_to_evaluated(value, options, 0)?
        }
        Encoding::Toml => {
            let input = str::from_utf8(bytes)?;
            toml_to_evaluated(toml::from_str(input)?, options, 0)?
        }
        Encoding::Cue => {
            return Err(DecodeError::Unsupported(
                "CUE source decoding is handled by the syntax/compiler pipeline".to_owned(),
            ));
        }
    };
    Ok(Value::from_evaluated(evaluated))
}

/// Encodes a value into the requested external data format.
///
/// # Errors
///
/// Returns [`EncodeError`] when evaluation fails or the value is not concrete.
pub fn encode_value(value: &Value, options: EncodeOptions) -> Result<String, EncodeError> {
    let evaluated = if options.concrete {
        let mut validate_options = ValidateOptions::default();
        validate_options.all_errors = true;
        let exported = value
            .evaluate_export(options.export_options)?
            .resolve_defaults();
        Value::from_evaluated(exported.clone()).validate(validate_options)?;
        exported
    } else {
        value.evaluate_export(options.export_options)?
    };
    match options.encoding {
        Encoding::Cue => Ok(format_cue_value(&evaluated)),
        Encoding::Json => Ok(serde_json::to_string_pretty(&evaluated_to_json(
            evaluated,
        )?)?),
        Encoding::Yaml => {
            let yaml = evaluated_to_yaml(evaluated)?;
            noyalib::to_string(&yaml).map_err(|error| EncodeError::Yaml(error.to_string()))
        }
        Encoding::Toml => {
            let toml = evaluated_to_toml(evaluated)?;
            Ok(toml::to_string_pretty(&toml)?)
        }
    }
}

fn validate_size(bytes: &[u8], limits: SourceLimits) -> Result<(), DecodeError> {
    if bytes.len() > limits.max_file_bytes() {
        return Err(DecodeError::SourceTooLarge {
            actual: bytes.len(),
            limit: limits.max_file_bytes(),
        });
    }
    Ok(())
}

fn json_to_evaluated(
    value: JsonValue,
    options: DecodeOptions,
    depth: u32,
) -> Result<EvaluatedValue, DecodeError> {
    validate_depth(depth, options)?;
    match value {
        JsonValue::Null => Ok(EvaluatedValue::Null),
        JsonValue::Bool(value) => Ok(EvaluatedValue::Bool(value)),
        JsonValue::Number(value) => Ok(EvaluatedValue::Number(value.to_string())),
        JsonValue::String(value) => {
            validate_string(&value, options)?;
            Ok(EvaluatedValue::String(value))
        }
        JsonValue::Array(values) => {
            validate_collection(values.len(), options)?;
            values
                .into_iter()
                .map(|value| json_to_evaluated(value, options, depth.saturating_add(1)))
                .collect::<Result<Vec<_>, _>>()
                .map(EvaluatedValue::List)
        }
        JsonValue::Object(values) => {
            validate_collection(values.len(), options)?;
            values
                .into_iter()
                .map(|(key, value)| {
                    validate_string(&key, options)?;
                    json_to_evaluated(value, options, depth.saturating_add(1))
                        .map(|value| (key, value))
                })
                .collect::<Result<indexmap::IndexMap<_, _>, _>>()
                .map(EvaluatedValue::Struct)
        }
    }
}

fn toml_to_evaluated(
    value: TomlValue,
    options: DecodeOptions,
    depth: u32,
) -> Result<EvaluatedValue, DecodeError> {
    validate_depth(depth, options)?;
    match value {
        TomlValue::String(value) => {
            validate_string(&value, options)?;
            Ok(EvaluatedValue::String(value))
        }
        TomlValue::Integer(value) => Ok(EvaluatedValue::Number(value.to_string())),
        TomlValue::Float(value) => Ok(EvaluatedValue::Number(value.to_string())),
        TomlValue::Boolean(value) => Ok(EvaluatedValue::Bool(value)),
        TomlValue::Datetime(value) => Ok(EvaluatedValue::String(value.to_string())),
        TomlValue::Array(values) => {
            validate_collection(values.len(), options)?;
            values
                .into_iter()
                .map(|value| toml_to_evaluated(value, options, depth.saturating_add(1)))
                .collect::<Result<Vec<_>, _>>()
                .map(EvaluatedValue::List)
        }
        TomlValue::Table(values) => {
            validate_collection(values.len(), options)?;
            values
                .into_iter()
                .map(|(key, value)| {
                    validate_string(&key, options)?;
                    toml_to_evaluated(value, options, depth.saturating_add(1))
                        .map(|value| (key, value))
                })
                .collect::<Result<indexmap::IndexMap<_, _>, _>>()
                .map(EvaluatedValue::Struct)
        }
    }
}

fn validate_depth(depth: u32, options: DecodeOptions) -> Result<(), DecodeError> {
    if depth > options.max_depth {
        return Err(DecodeError::MaxDepth {
            limit: options.max_depth,
        });
    }
    Ok(())
}

fn validate_collection(actual: usize, options: DecodeOptions) -> Result<(), DecodeError> {
    if actual > options.max_collection_items {
        return Err(DecodeError::MaxCollectionItems {
            actual,
            limit: options.max_collection_items,
        });
    }
    Ok(())
}

fn validate_string(value: &str, options: DecodeOptions) -> Result<(), DecodeError> {
    let actual = value.len();
    if actual > options.max_string_bytes {
        return Err(DecodeError::MaxStringBytes {
            actual,
            limit: options.max_string_bytes,
        });
    }
    Ok(())
}

fn evaluated_to_json(value: EvaluatedValue) -> Result<JsonValue, EncodeError> {
    match value {
        EvaluatedValue::Top => unsupported(Encoding::Json, "incomplete value"),
        EvaluatedValue::Null => Ok(JsonValue::Null),
        EvaluatedValue::Bool(value) => Ok(JsonValue::Bool(value)),
        EvaluatedValue::Number(value) => JsonNumber::from_str(normalized_number(&value).as_ref())
            .map(JsonValue::Number)
            .map_err(|_| unsupported_error(Encoding::Json, "invalid JSON number")),
        EvaluatedValue::String(value) => Ok(JsonValue::String(value)),
        EvaluatedValue::Bytes(_) => unsupported(Encoding::Json, "bytes require binary encoding"),
        EvaluatedValue::Struct(values)
        | EvaluatedValue::PatternedStruct { fields: values, .. }
        | EvaluatedValue::ClosedStruct(values)
        | EvaluatedValue::ClosedPatternedStruct { fields: values, .. } => {
            let mut object = JsonMap::new();
            for (key, value) in values {
                object.insert(key, evaluated_to_json(value)?);
            }
            Ok(JsonValue::Object(object))
        }
        EvaluatedValue::List(values) => values
            .into_iter()
            .map(evaluated_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(JsonValue::Array),
        EvaluatedValue::OpenList { .. } => unsupported(Encoding::Json, "incomplete open list"),
        EvaluatedValue::Kind(_) => unsupported(Encoding::Json, "incomplete kind constraint"),
        EvaluatedValue::Builtin(_) => unsupported(Encoding::Json, "incomplete builtin value"),
        EvaluatedValue::NumericConstraint(_)
        | EvaluatedValue::NumberMultipleOf(_)
        | EvaluatedValue::NumberConstraintSet { .. } => {
            unsupported(Encoding::Json, "incomplete numeric constraint")
        }
        EvaluatedValue::StringConstraints(_) | EvaluatedValue::StringConstraintSet(_) => {
            unsupported(Encoding::Json, "incomplete string constraint")
        }
        EvaluatedValue::RegexConstraint { .. } => {
            unsupported(Encoding::Json, "incomplete regex constraint")
        }
        EvaluatedValue::Default(value) => evaluated_to_json(*value),
        EvaluatedValue::OptionalField(_) => unsupported(
            Encoding::Json,
            "optional field constraint is not concrete data",
        ),
        EvaluatedValue::Disjunction(_) => unsupported(Encoding::Json, "incomplete disjunction"),
        EvaluatedValue::ComprehensionItems(_) => {
            unsupported(Encoding::Json, "unmaterialized comprehension items")
        }
        EvaluatedValue::Bottom(bottom) => unsupported(Encoding::Json, bottom.message),
        _ => unsupported(Encoding::Json, "unsupported value"),
    }
}

fn evaluated_to_toml(value: EvaluatedValue) -> Result<TomlValue, EncodeError> {
    match value {
        EvaluatedValue::Top => unsupported(Encoding::Toml, "incomplete value"),
        EvaluatedValue::Null => unsupported(Encoding::Toml, "TOML has no null value"),
        EvaluatedValue::Bool(value) => Ok(TomlValue::Boolean(value)),
        EvaluatedValue::Number(value) => number_to_toml(&value),
        EvaluatedValue::String(value) => Ok(TomlValue::String(value)),
        EvaluatedValue::Bytes(_) => unsupported(Encoding::Toml, "bytes require binary encoding"),
        EvaluatedValue::Struct(values)
        | EvaluatedValue::PatternedStruct { fields: values, .. }
        | EvaluatedValue::ClosedStruct(values)
        | EvaluatedValue::ClosedPatternedStruct { fields: values, .. } => {
            let mut table = TomlTable::new();
            for (key, value) in values {
                table.insert(key, evaluated_to_toml(value)?);
            }
            Ok(TomlValue::Table(table))
        }
        EvaluatedValue::List(values) => values
            .into_iter()
            .map(evaluated_to_toml)
            .collect::<Result<Vec<_>, _>>()
            .map(TomlValue::Array),
        EvaluatedValue::OpenList { .. } => unsupported(Encoding::Toml, "incomplete open list"),
        EvaluatedValue::Kind(_) => unsupported(Encoding::Toml, "incomplete kind constraint"),
        EvaluatedValue::Builtin(_) => unsupported(Encoding::Toml, "incomplete builtin value"),
        EvaluatedValue::NumericConstraint(_)
        | EvaluatedValue::NumberMultipleOf(_)
        | EvaluatedValue::NumberConstraintSet { .. } => {
            unsupported(Encoding::Toml, "incomplete numeric constraint")
        }
        EvaluatedValue::StringConstraints(_) | EvaluatedValue::StringConstraintSet(_) => {
            unsupported(Encoding::Toml, "incomplete string constraint")
        }
        EvaluatedValue::RegexConstraint { .. } => {
            unsupported(Encoding::Toml, "incomplete regex constraint")
        }
        EvaluatedValue::Default(value) => evaluated_to_toml(*value),
        EvaluatedValue::OptionalField(_) => unsupported(
            Encoding::Toml,
            "optional field constraint is not concrete data",
        ),
        EvaluatedValue::Disjunction(_) => unsupported(Encoding::Toml, "incomplete disjunction"),
        EvaluatedValue::ComprehensionItems(_) => {
            unsupported(Encoding::Toml, "unmaterialized comprehension items")
        }
        EvaluatedValue::Bottom(bottom) => unsupported(Encoding::Toml, bottom.message),
        _ => unsupported(Encoding::Toml, "unsupported value"),
    }
}

fn evaluated_to_yaml(value: EvaluatedValue) -> Result<YamlValue, EncodeError> {
    match value {
        EvaluatedValue::Top => unsupported(Encoding::Yaml, "incomplete value"),
        EvaluatedValue::Null => Ok(YamlValue::Null),
        EvaluatedValue::Bool(value) => Ok(YamlValue::Bool(value)),
        EvaluatedValue::Number(value) => number_to_yaml(&value),
        EvaluatedValue::String(value) => Ok(YamlValue::String(value)),
        EvaluatedValue::Bytes(_) => unsupported(Encoding::Yaml, "bytes require binary encoding"),
        EvaluatedValue::Struct(values)
        | EvaluatedValue::PatternedStruct { fields: values, .. }
        | EvaluatedValue::ClosedStruct(values)
        | EvaluatedValue::ClosedPatternedStruct { fields: values, .. } => {
            let entries = values
                .into_iter()
                .map(|(key, value)| evaluated_to_yaml(value).map(|value| (key, value)))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(YamlValue::Mapping(YamlMapping::from(entries)))
        }
        EvaluatedValue::List(values) => values
            .into_iter()
            .map(evaluated_to_yaml)
            .collect::<Result<Vec<_>, _>>()
            .map(YamlValue::Sequence),
        EvaluatedValue::OpenList { .. } => unsupported(Encoding::Yaml, "incomplete open list"),
        EvaluatedValue::Kind(_) => unsupported(Encoding::Yaml, "incomplete kind constraint"),
        EvaluatedValue::Builtin(_) => unsupported(Encoding::Yaml, "incomplete builtin value"),
        EvaluatedValue::NumericConstraint(_)
        | EvaluatedValue::NumberMultipleOf(_)
        | EvaluatedValue::NumberConstraintSet { .. } => {
            unsupported(Encoding::Yaml, "incomplete numeric constraint")
        }
        EvaluatedValue::StringConstraints(_) | EvaluatedValue::StringConstraintSet(_) => {
            unsupported(Encoding::Yaml, "incomplete string constraint")
        }
        EvaluatedValue::RegexConstraint { .. } => {
            unsupported(Encoding::Yaml, "incomplete regex constraint")
        }
        EvaluatedValue::Default(value) => evaluated_to_yaml(*value),
        EvaluatedValue::OptionalField(_) => unsupported(
            Encoding::Yaml,
            "optional field constraint is not concrete data",
        ),
        EvaluatedValue::Disjunction(_) => unsupported(Encoding::Yaml, "incomplete disjunction"),
        EvaluatedValue::ComprehensionItems(_) => {
            unsupported(Encoding::Yaml, "unmaterialized comprehension items")
        }
        EvaluatedValue::Bottom(bottom) => unsupported(Encoding::Yaml, bottom.message),
        _ => unsupported(Encoding::Yaml, "unsupported value"),
    }
}

fn number_to_toml(value: &str) -> Result<TomlValue, EncodeError> {
    let normalized = normalized_number(value);
    if let Ok(integer) = normalized.parse::<i64>() {
        return Ok(TomlValue::Integer(integer));
    }
    if let Some(float) = exact_f64(&normalized) {
        return Ok(TomlValue::Float(float));
    }
    unsupported(
        Encoding::Toml,
        format!("number `{value}` cannot be represented exactly as TOML"),
    )
}

fn number_to_yaml(value: &str) -> Result<YamlValue, EncodeError> {
    let normalized = normalized_number(value);
    if let Ok(integer) = normalized.parse::<i64>() {
        return Ok(YamlValue::Number(YamlNumber::Integer(integer)));
    }
    if let Some(float) = exact_f64(&normalized) {
        return Ok(YamlValue::Number(YamlNumber::Float(float)));
    }
    unsupported(
        Encoding::Yaml,
        format!("number `{value}` cannot be represented exactly as YAML"),
    )
}

fn normalized_number(value: &str) -> Cow<'_, str> {
    if value.contains('_') {
        Cow::Owned(value.replace('_', ""))
    } else {
        Cow::Borrowed(value)
    }
}

fn exact_f64(value: &str) -> Option<f64> {
    let float = value.parse::<f64>().ok()?;
    if !float.is_finite() {
        return None;
    }
    let original = BigDecimal::from_str(value).ok()?;
    let rendered = BigDecimal::from_str(&float.to_string()).ok()?;
    (original == rendered).then_some(float)
}

fn unsupported<T>(encoding: Encoding, message: impl Into<String>) -> Result<T, EncodeError> {
    Err(unsupported_error(encoding, message))
}

fn unsupported_error(encoding: Encoding, message: impl Into<String>) -> EncodeError {
    EncodeError::Unsupported {
        encoding,
        message: message.into(),
    }
}

fn format_cue_value(value: &EvaluatedValue) -> String {
    match value {
        EvaluatedValue::Top => "_".to_owned(),
        EvaluatedValue::Null => "null".to_owned(),
        EvaluatedValue::Bool(value) => value.to_string(),
        EvaluatedValue::Number(value) => value.clone(),
        EvaluatedValue::String(value) => format!("{value:?}"),
        EvaluatedValue::Bytes(value) => format_cue_bytes(value),
        EvaluatedValue::Struct(fields)
        | EvaluatedValue::PatternedStruct { fields, .. }
        | EvaluatedValue::ClosedStruct(fields)
        | EvaluatedValue::ClosedPatternedStruct { fields, .. } => format_cue_struct(fields),
        EvaluatedValue::List(values) => {
            let rendered = values
                .iter()
                .map(format_cue_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{rendered}]")
        }
        EvaluatedValue::OpenList { items, tail } => format_cue_open_list(items, tail),
        EvaluatedValue::Kind(kind) => kind.to_string(),
        EvaluatedValue::Builtin(name) => name.clone(),
        EvaluatedValue::NumericConstraint(bounds) => format_cue_numeric_bounds(bounds),
        EvaluatedValue::NumberMultipleOf(multiples) => {
            format_cue_multiple_of_constraints(multiples)
        }
        EvaluatedValue::NumberConstraintSet { bounds, multiples } => {
            format_cue_number_constraint_set(bounds, multiples)
        }
        EvaluatedValue::StringConstraints(constraints) => constraints
            .iter()
            .map(format_cue_string_constraint)
            .collect::<Vec<_>>()
            .join(" & "),
        EvaluatedValue::StringConstraintSet(constraints) => {
            format_cue_string_constraint_set(constraints)
        }
        EvaluatedValue::RegexConstraint { pattern, negated } => {
            let op = if *negated { "!~" } else { "=~" };
            format!("{op}{pattern:?}")
        }
        EvaluatedValue::Default(value) => format!("*{}", format_cue_value(value)),
        EvaluatedValue::OptionalField(value) => format_cue_value(value),
        EvaluatedValue::Disjunction(disjuncts) => disjuncts
            .iter()
            .map(|disjunct| {
                let value = format_cue_value(&disjunct.value);
                if disjunct.default {
                    format!("*{value}")
                } else {
                    value
                }
            })
            .collect::<Vec<_>>()
            .join(" | "),
        EvaluatedValue::ComprehensionItems(items) => {
            let rendered = items
                .iter()
                .map(format_cue_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{rendered}]")
        }
        EvaluatedValue::Bottom(bottom) => format!("_|_({:?})", bottom.message),
        _ => "_|_(\"unsupported value\")".to_owned(),
    }
}

fn format_cue_string_constraint(constraint: &StringConstraint) -> String {
    format!("{}({})", constraint.op.builtin_name(), constraint.limit)
}

fn format_cue_numeric_bounds(bounds: &[NumericBound]) -> String {
    bounds
        .iter()
        .map(|bound| {
            let op = bound.op.as_str();
            format!("{op}{}", bound.value)
        })
        .collect::<Vec<_>>()
        .join(" & ")
}

fn format_cue_multiple_of_constraints(multiples: &[String]) -> String {
    multiples
        .iter()
        .map(|divisor| format!("math.MultipleOf({divisor})"))
        .collect::<Vec<_>>()
        .join(" & ")
}

fn format_cue_number_constraint_set(bounds: &[NumericBound], multiples: &[String]) -> String {
    let mut rendered = Vec::new();
    if !bounds.is_empty() {
        rendered.push(format_cue_numeric_bounds(bounds));
    }
    if !multiples.is_empty() {
        rendered.push(format_cue_multiple_of_constraints(multiples));
    }
    rendered.join(" & ")
}

fn format_cue_string_constraint_set(constraints: &StringConstraintSet) -> String {
    let mut rendered = constraints
        .runes
        .iter()
        .map(format_cue_string_constraint)
        .collect::<Vec<_>>();
    rendered.extend(constraints.regexes.iter().map(|regex| {
        let op = if regex.negated { "!~" } else { "=~" };
        format!("{op}{:?}", regex.pattern)
    }));
    rendered.join(" & ")
}

fn format_cue_open_list(items: &[EvaluatedValue], tail: &EvaluatedValue) -> String {
    let mut rendered = items.iter().map(format_cue_value).collect::<Vec<_>>();
    if matches!(tail, EvaluatedValue::Top) {
        rendered.push("...".to_owned());
    } else {
        rendered.push(format!("...{}", format_cue_value(tail)));
    }
    format!("[{}]", rendered.join(", "))
}

fn format_cue_bytes(value: &[u8]) -> String {
    let mut rendered = String::from("'");
    for byte in value {
        match *byte {
            b'\'' => rendered.push_str("\\'"),
            b'\\' => rendered.push_str("\\\\"),
            b'\n' => rendered.push_str("\\n"),
            b'\r' => rendered.push_str("\\r"),
            b'\t' => rendered.push_str("\\t"),
            b' '..=b'~' => rendered.push(char::from(*byte)),
            byte => {
                rendered.push_str("\\x");
                rendered.push(hex_digit(byte >> 4));
                rendered.push(hex_digit(byte & 0x0f));
            }
        }
    }
    rendered.push('\'');
    rendered
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + (value - 10)),
        _ => '?',
    }
}

fn format_cue_struct(fields: &indexmap::IndexMap<String, EvaluatedValue>) -> String {
    if fields.is_empty() {
        return "{}".to_owned();
    }
    let body = fields
        .iter()
        .map(|(key, value)| {
            if let EvaluatedValue::OptionalField(value) = value {
                format!("{key}?: {}", format_cue_value(value))
            } else {
                format!("{key}: {}", format_cue_value(value))
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{body}}}")
}

#[cfg(test)]
mod tests {
    use cue_rust_eval::{EvaluatedValue, Value};

    use super::{DecodeOptions, EncodeOptions, Encoding, decode_bytes, encode_value};

    #[test]
    fn test_should_decode_json_and_encode_cue() -> Result<(), Box<dyn std::error::Error>> {
        let value = decode_bytes(Encoding::Json, br#"{"x":1}"#, DecodeOptions::default())?;
        assert_eq!(
            "{x: 1}",
            encode_value(
                &value,
                EncodeOptions {
                    encoding: Encoding::Cue,
                    concrete: true,
                    ..EncodeOptions::default()
                }
            )?
        );
        Ok(())
    }

    #[test]
    fn test_should_decode_yaml_and_encode_json() -> Result<(), Box<dyn std::error::Error>> {
        let value = decode_bytes(Encoding::Yaml, b"x: 1\n", DecodeOptions::default())?;
        let output = encode_value(&value, EncodeOptions::default())?;
        assert!(output.contains("\"x\": 1"));
        Ok(())
    }

    #[test]
    fn test_should_decode_toml_and_encode_json() -> Result<(), Box<dyn std::error::Error>> {
        let value = decode_bytes(Encoding::Toml, b"x = 1\n", DecodeOptions::default())?;
        let output = encode_value(&value, EncodeOptions::default())?;
        assert!(output.contains("\"x\": 1"));
        Ok(())
    }

    #[test]
    fn test_should_encode_bytes_as_cue_literal() -> Result<(), Box<dyn std::error::Error>> {
        let value = Value::from_evaluated(EvaluatedValue::Bytes(b"a\n\xff".to_vec()));
        assert_eq!(
            "'a\\n\\xff'",
            encode_value(
                &value,
                EncodeOptions {
                    encoding: Encoding::Cue,
                    concrete: true,
                    ..EncodeOptions::default()
                },
            )?,
        );
        Ok(())
    }

    #[test]
    fn test_should_reject_inexact_yaml_and_toml_numbers() -> Result<(), Box<dyn std::error::Error>>
    {
        fn number_field(number: &str) -> Value {
            Value::from_evaluated(EvaluatedValue::Struct(indexmap::IndexMap::from([(
                "n".to_owned(),
                EvaluatedValue::Number(number.to_owned()),
            )])))
        }

        let exact = number_field("0.1");
        for encoding in [Encoding::Yaml, Encoding::Toml] {
            let output = encode_value(
                &exact,
                EncodeOptions {
                    encoding,
                    ..EncodeOptions::default()
                },
            )?;
            assert!(output.contains("0.1"));
        }

        for number in ["9223372036854775808", "1.234567890123456789"] {
            let value = number_field(number);
            for encoding in [Encoding::Yaml, Encoding::Toml] {
                let result = encode_value(
                    &value,
                    EncodeOptions {
                        encoding,
                        ..EncodeOptions::default()
                    },
                );
                assert!(
                    matches!(result, Err(super::EncodeError::Unsupported { .. })),
                    "{encoding:?} unexpectedly accepted {number}: {result:?}",
                );
            }
        }
        Ok(())
    }

    #[test]
    fn test_should_encode_underscored_numbers_as_external_numbers()
    -> Result<(), Box<dyn std::error::Error>> {
        let value = Value::from_evaluated(EvaluatedValue::Struct(indexmap::IndexMap::from([(
            "x".to_owned(),
            EvaluatedValue::Number("1_000".to_owned()),
        )])));

        let json = encode_value(&value, EncodeOptions::default())?;
        assert!(json.contains("\"x\": 1000"));

        for encoding in [Encoding::Yaml, Encoding::Toml] {
            let output = encode_value(
                &value,
                EncodeOptions {
                    encoding,
                    ..EncodeOptions::default()
                },
            )?;
            assert!(
                output.contains("1000"),
                "{encoding:?} did not normalize numeric separator: {output}",
            );
        }
        Ok(())
    }

    #[test]
    fn test_should_reject_decoder_depth_limit() {
        let options = DecodeOptions {
            max_depth: 0,
            ..DecodeOptions::default()
        };
        let result = decode_bytes(Encoding::Json, br#"{"x":1}"#, options);
        assert!(matches!(result, Err(super::DecodeError::MaxDepth { .. })));
    }

    #[test]
    fn test_should_reject_decoder_string_limit() {
        let options = DecodeOptions {
            max_string_bytes: 1,
            ..DecodeOptions::default()
        };
        let result = decode_bytes(Encoding::Json, br#""long""#, options);
        assert!(matches!(
            result,
            Err(super::DecodeError::MaxStringBytes { .. })
        ));
    }
}
