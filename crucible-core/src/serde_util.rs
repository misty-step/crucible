//! Internal serde guards shared by the versioned persisted artifacts.
//!
//! Two boundary checks the schema-stamped artifacts (`EvaluationCard`, `Label`,
//! `CalibrationRecord`, `JudgmentQueue`, `EvalSpec`) apply at their serde edge:
//!
//! - `serialize_finite` / `serialize_finite_pair` refuse a non-finite `f64` on
//!   serialize. `serde_json` renders `NaN`/`±∞` as JSON `null`, which then fails
//!   to deserialize back into an `f64` — a silent round-trip break. The measure
//!   layer never emits a non-finite rate today, so this is defense in depth: it
//!   fails loudly rather than ever writing an artifact that cannot be read back.
//! - `expect_schema` validates a `schema_version` against the one known value on
//!   deserialize, rejecting an unknown/garbage version with a clear error instead
//!   of silently treating any string as the current schema. Paired with
//!   `#[serde(default = ...)]`, an *absent* field still defaults to the known
//!   schema, so an older artifact that predates the field still loads.

use serde::{ser::SerializeTuple, Deserialize, Deserializer, Serializer};

/// Serialize an `f64` that must be finite, erroring rather than emitting JSON
/// `null` for a `NaN`/`±∞` (which would not deserialize back into an `f64`).
pub(crate) fn serialize_finite<S>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if value.is_finite() {
        serializer.serialize_f64(*value)
    } else {
        Err(serde::ser::Error::custom(format!(
            "non-finite f64 ({value}) cannot be written to a versioned artifact"
        )))
    }
}

/// `serialize_finite` for a `(f64, f64)` pair — e.g. a confidence interval —
/// erroring if either component is non-finite. Emits the same JSON array shape
/// as the derived tuple serialization.
pub(crate) fn serialize_finite_pair<S>(value: &(f64, f64), serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let (a, b) = *value;
    if !a.is_finite() || !b.is_finite() {
        return Err(serde::ser::Error::custom(format!(
            "non-finite f64 in pair ({a}, {b}) cannot be written to a versioned artifact"
        )));
    }
    let mut tuple = serializer.serialize_tuple(2)?;
    tuple.serialize_element(&a)?;
    tuple.serialize_element(&b)?;
    tuple.end()
}

/// Deserialize a `schema_version`, accepting only `expected` and rejecting any
/// other value with a clear error. Paired with `#[serde(default = ...)]` so an
/// absent field still defaults to the known schema rather than reaching here.
pub(crate) fn expect_schema<'de, D>(deserializer: D, expected: &str) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let got = String::deserialize(deserializer)?;
    if got == expected {
        Ok(got)
    } else {
        Err(serde::de::Error::custom(format!(
            "unknown schema_version {got:?}: expected {expected:?}"
        )))
    }
}
