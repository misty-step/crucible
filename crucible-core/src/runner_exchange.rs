//! Language-neutral runner exchange artifacts.
//!
//! Crucible owns evaluation identity, evidence, trust, and compatibility, but
//! it does not need every useful runner in its Rust process. These envelopes
//! are the strict JSON waist between Crucible and an external runner. Adapter-
//! specific data stays in `adapter_payload`; additive top-level fields are
//! retained in `extra`. Execution transport is deliberately out of scope for
//! this module: a subprocess, container, or remote worker can all speak the
//! same request/result contract.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

/// Schema identifier for an external-runner request.
pub const RUNNER_EXCHANGE_REQUEST_SCHEMA: &str = "crucible.runner_exchange_request.v1";
/// Schema identifier for an external-runner result.
pub const RUNNER_EXCHANGE_RESULT_SCHEMA: &str = "crucible.runner_exchange_result.v1";

/// A machine-readable conformance error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeValidationError {
    /// Stable error category for callers and fixtures.
    pub code: String,
    /// Dotted field path that failed validation.
    pub field: String,
    /// Human-readable remediation detail.
    pub message: String,
}

impl ExchangeValidationError {
    fn new(code: &str, field: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            field: field.to_string(),
            message: message.into(),
        }
    }
}

/// External adapter identity, bound to the artifact used for this exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterIdentity {
    pub name: String,
    pub version: String,
    pub digest: String,
}

/// Candidate family, including deterministic non-model controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Model,
    Agent,
    Deterministic,
}

/// Requested model identity. The actual response model is result provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeModelIdentity {
    pub provider: String,
    pub name: String,
}

/// Candidate configuration echoed unchanged by the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateIdentity {
    pub kind: CandidateKind,
    pub harness: String,
    pub harness_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ExchangeModelIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolset_hash: Option<String>,
}

/// Task/workspace references relative to the exchange root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeInput {
    pub workspace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default)]
    pub context: Vec<String>,
    pub digest: String,
}

/// Network authority granted to the runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAuthority {
    Deny,
    Allowlist,
}

/// Explicit authority; credential values never belong here, only references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeAuthority {
    #[serde(default)]
    pub filesystem_roots: Vec<String>,
    pub network: NetworkAuthority,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub credential_refs: Vec<String>,
}

/// Deterministic resource and output bounds the transport must enforce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeLimits {
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
    pub cpu_millicores: u32,
    pub memory_mb: u32,
    pub storage_mb: u32,
}

/// Caller provenance for the requested task snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeRequestProvenance {
    pub repo: String,
    pub git_sha: String,
    pub invocation_id: String,
}

/// Evidence required before Crucible may trust a successful result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeTrustRequirements {
    #[serde(default)]
    pub required_evidence_kinds: Vec<String>,
    #[serde(default)]
    pub require_transcript: bool,
    #[serde(default)]
    pub require_usage: bool,
    /// Require an explicit cost value (zero is valid) rather than unknown cost.
    #[serde(default)]
    pub require_cost: bool,
    /// Require the adapter to report the provider's actual response model.
    #[serde(default)]
    pub require_response_model: bool,
}

/// One external execution request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerExchangeRequest {
    #[serde(deserialize_with = "deserialize_request_schema")]
    pub schema_version: String,
    pub exchange_id: String,
    pub task_id: String,
    pub adapter: AdapterIdentity,
    pub candidate: CandidateIdentity,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub input: ExchangeInput,
    pub authority: ExchangeAuthority,
    pub limits: ExchangeLimits,
    pub provenance: ExchangeRequestProvenance,
    pub trust: ExchangeTrustRequirements,
    #[serde(default)]
    pub adapter_payload: Value,
    /// Additive compatible fields survive deserialize/serialize unchanged.
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Terminal outcome of the external process contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExchangeStatus {
    Success,
    Refused,
    Timeout,
    MalformedOutput,
    ExecutionError,
}

/// Primary candidate output, referenced rather than embedded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExchangeOutput {
    #[serde(default)]
    pub summary: String,
    pub primary_artifact: String,
    #[serde(default)]
    pub metadata: Value,
}

/// Content-addressed evidence relative to the exchange result root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceReference {
    pub kind: String,
    pub path: String,
    /// Lowercase hexadecimal SHA-256 digest without a prefix.
    pub sha256: String,
    pub media_type: String,
}

/// Usage and economic receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExchangeUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub latency_ms: u64,
}

/// Structured non-success detail. Policy keys off fields, never prose parsing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExchangeError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub detail: Value,
}

/// Runtime provenance observed by the adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeResultProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    pub started_at: String,
    pub finished_at: String,
}

/// One external execution result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerExchangeResult {
    #[serde(deserialize_with = "deserialize_result_schema")]
    pub schema_version: String,
    pub exchange_id: String,
    pub adapter: AdapterIdentity,
    pub candidate: CandidateIdentity,
    pub status: ExchangeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<ExchangeOutput>,
    #[serde(default)]
    pub evidence: Vec<EvidenceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ExchangeUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ExchangeError>,
    pub provenance: ExchangeResultProvenance,
    #[serde(default)]
    pub adapter_payload: Value,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl RunnerExchangeRequest {
    /// Validate the internally deterministic portion of a request.
    pub fn validate(&self) -> Result<(), Vec<ExchangeValidationError>> {
        let mut errors = Vec::new();
        validate_schema(
            &mut errors,
            "schema_version",
            &self.schema_version,
            RUNNER_EXCHANGE_REQUEST_SCHEMA,
        );
        validate_nonempty(&mut errors, "exchange_id", &self.exchange_id);
        validate_nonempty(&mut errors, "task_id", &self.task_id);
        validate_adapter(&mut errors, "adapter", &self.adapter);
        validate_candidate(&mut errors, "candidate", &self.candidate);
        if self.capabilities.is_empty() {
            errors.push(ExchangeValidationError::new(
                "missing_capability",
                "capabilities",
                "at least one declared capability is required",
            ));
        }
        validate_unique_nonempty(&mut errors, "capabilities", &self.capabilities);
        validate_relative(&mut errors, "input.workspace", &self.input.workspace);
        if let Some(path) = &self.input.instruction {
            validate_relative(&mut errors, "input.instruction", path);
        }
        for (index, path) in self.input.context.iter().enumerate() {
            validate_relative(&mut errors, &format!("input.context[{index}]"), path);
        }
        validate_prefixed_digest(&mut errors, "input.digest", &self.input.digest);

        if self.authority.filesystem_roots.is_empty() {
            errors.push(ExchangeValidationError::new(
                "missing_authority",
                "authority.filesystem_roots",
                "at least one confined filesystem root is required",
            ));
        }
        for (index, path) in self.authority.filesystem_roots.iter().enumerate() {
            validate_relative(
                &mut errors,
                &format!("authority.filesystem_roots[{index}]"),
                path,
            );
        }
        for (field, path) in std::iter::once(("input.workspace".to_string(), &self.input.workspace))
            .chain(
                self.input
                    .instruction
                    .as_ref()
                    .map(|path| ("input.instruction".to_string(), path)),
            )
            .chain(
                self.input
                    .context
                    .iter()
                    .enumerate()
                    .map(|(index, path)| (format!("input.context[{index}]"), path)),
            )
        {
            if !authority_covers(path, &self.authority.filesystem_roots) {
                errors.push(ExchangeValidationError::new(
                    "ungranted_path",
                    &field,
                    "input path must be covered by authority.filesystem_roots",
                ));
            }
        }
        match self.authority.network {
            NetworkAuthority::Deny if !self.authority.allowed_hosts.is_empty() => {
                errors.push(ExchangeValidationError::new(
                    "contradictory_authority",
                    "authority.allowed_hosts",
                    "network=deny cannot declare allowed hosts",
                ))
            }
            NetworkAuthority::Allowlist if self.authority.allowed_hosts.is_empty() => {
                errors.push(ExchangeValidationError::new(
                    "missing_authority",
                    "authority.allowed_hosts",
                    "network=allowlist requires at least one host",
                ))
            }
            _ => {}
        }
        validate_unique_nonempty(
            &mut errors,
            "authority.allowed_hosts",
            &self.authority.allowed_hosts,
        );
        validate_unique_nonempty(
            &mut errors,
            "authority.credential_refs",
            &self.authority.credential_refs,
        );
        for (index, reference) in self.authority.credential_refs.iter().enumerate() {
            if !is_credential_reference(reference) {
                errors.push(ExchangeValidationError::new(
                    "invalid_credential_reference",
                    &format!("authority.credential_refs[{index}]"),
                    "credentials must be named as ref:<broker-path>, never embedded values",
                ));
            }
        }
        for (index, host) in self.authority.allowed_hosts.iter().enumerate() {
            if host.contains(['/', '@', ':']) || host.chars().any(char::is_whitespace) {
                errors.push(ExchangeValidationError::new(
                    "invalid_host",
                    &format!("authority.allowed_hosts[{index}]"),
                    "allowed hosts are bare hostnames, not URLs or credentials",
                ));
            }
        }

        for (field, value) in [
            ("limits.timeout_ms", self.limits.timeout_ms),
            ("limits.max_output_bytes", self.limits.max_output_bytes),
            (
                "limits.cpu_millicores",
                u64::from(self.limits.cpu_millicores),
            ),
            ("limits.memory_mb", u64::from(self.limits.memory_mb)),
            ("limits.storage_mb", u64::from(self.limits.storage_mb)),
        ] {
            if value == 0 {
                invalid_positive(&mut errors, field);
            }
        }
        validate_nonempty(&mut errors, "provenance.repo", &self.provenance.repo);
        if !is_hex_len(&self.provenance.git_sha, 40) {
            errors.push(ExchangeValidationError::new(
                "invalid_provenance",
                "provenance.git_sha",
                "git_sha must be a 40-character lowercase hexadecimal object id",
            ));
        }
        validate_nonempty(
            &mut errors,
            "provenance.invocation_id",
            &self.provenance.invocation_id,
        );
        validate_unique_nonempty(
            &mut errors,
            "trust.required_evidence_kinds",
            &self.trust.required_evidence_kinds,
        );
        if self.trust.require_transcript
            && !self
                .trust
                .required_evidence_kinds
                .iter()
                .any(|kind| kind == "transcript")
        {
            errors.push(ExchangeValidationError::new(
                "contradictory_trust",
                "trust.required_evidence_kinds",
                "require_transcript=true requires transcript evidence",
            ));
        }
        if self.trust.require_cost && !self.trust.require_usage {
            errors.push(ExchangeValidationError::new(
                "contradictory_trust",
                "trust.require_cost",
                "require_cost=true requires require_usage=true",
            ));
        }
        if self.trust.require_response_model && self.candidate.model.is_none() {
            errors.push(ExchangeValidationError::new(
                "contradictory_trust",
                "trust.require_response_model",
                "response-model identity cannot be required for a candidate without a model",
            ));
        }
        finish(errors)
    }
}

impl RunnerExchangeResult {
    /// Validate a result by itself and against the request it answers.
    pub fn validate_against(
        &self,
        request: &RunnerExchangeRequest,
    ) -> Result<(), Vec<ExchangeValidationError>> {
        let mut errors = match request.validate() {
            Ok(()) => Vec::new(),
            Err(request_errors) => request_errors
                .into_iter()
                .map(|error| ExchangeValidationError {
                    field: format!("request.{}", error.field),
                    ..error
                })
                .collect(),
        };
        validate_schema(
            &mut errors,
            "schema_version",
            &self.schema_version,
            RUNNER_EXCHANGE_RESULT_SCHEMA,
        );
        validate_nonempty(&mut errors, "exchange_id", &self.exchange_id);
        validate_adapter(&mut errors, "adapter", &self.adapter);
        validate_candidate(&mut errors, "candidate", &self.candidate);
        if self.exchange_id != request.exchange_id {
            identity_mismatch(&mut errors, "exchange_id");
        }
        if self.adapter != request.adapter {
            identity_mismatch(&mut errors, "adapter");
        }
        if self.candidate != request.candidate {
            identity_mismatch(&mut errors, "candidate");
        }

        match self.status {
            ExchangeStatus::Success => {
                if self.output.is_none() {
                    errors.push(ExchangeValidationError::new(
                        "missing_output",
                        "output",
                        "success requires a primary output",
                    ));
                }
                if self.error.is_some() {
                    errors.push(ExchangeValidationError::new(
                        "unexpected_error",
                        "error",
                        "success cannot carry an error",
                    ));
                }
            }
            _ => {
                if self.error.is_none() {
                    errors.push(ExchangeValidationError::new(
                        "missing_error",
                        "error",
                        "a non-success status requires a structured error",
                    ));
                }
                if self.output.is_some() {
                    errors.push(ExchangeValidationError::new(
                        "unexpected_output",
                        "output",
                        "a non-success status cannot claim a primary output",
                    ));
                }
            }
        }

        let mut evidence_kinds = BTreeSet::new();
        let mut evidence_paths = BTreeSet::new();
        for (index, evidence) in self.evidence.iter().enumerate() {
            let prefix = format!("evidence[{index}]");
            validate_nonempty(&mut errors, &format!("{prefix}.kind"), &evidence.kind);
            validate_relative(&mut errors, &format!("{prefix}.path"), &evidence.path);
            if !is_hex_len(&evidence.sha256, 64) {
                errors.push(ExchangeValidationError::new(
                    "invalid_digest",
                    &format!("{prefix}.sha256"),
                    "sha256 must be 64 lowercase hexadecimal characters",
                ));
            }
            validate_nonempty(
                &mut errors,
                &format!("{prefix}.media_type"),
                &evidence.media_type,
            );
            evidence_kinds.insert(evidence.kind.as_str());
            if !evidence_paths.insert(evidence.path.as_str()) {
                errors.push(ExchangeValidationError::new(
                    "duplicate_evidence",
                    &format!("{prefix}.path"),
                    "evidence paths must be unique",
                ));
            }
        }
        if let Some(output) = &self.output {
            validate_relative(
                &mut errors,
                "output.primary_artifact",
                &output.primary_artifact,
            );
            if !evidence_paths.contains(output.primary_artifact.as_str()) {
                errors.push(ExchangeValidationError::new(
                    "missing_evidence",
                    "output.primary_artifact",
                    "the primary artifact must have a content-addressed evidence entry",
                ));
            }
        }
        if self.status == ExchangeStatus::Success {
            for required in &request.trust.required_evidence_kinds {
                if !evidence_kinds.contains(required.as_str()) {
                    errors.push(ExchangeValidationError::new(
                        "missing_evidence",
                        "evidence",
                        format!("required evidence kind {required:?} is absent"),
                    ));
                }
            }
            if request.trust.require_usage && self.usage.is_none() {
                errors.push(ExchangeValidationError::new(
                    "missing_usage",
                    "usage",
                    "the request requires a usage receipt",
                ));
            }
            if request.trust.require_cost
                && self
                    .usage
                    .as_ref()
                    .is_none_or(|usage| usage.cost_usd.is_none())
            {
                errors.push(ExchangeValidationError::new(
                    "missing_usage",
                    "usage.cost_usd",
                    "the request requires an explicit cost receipt",
                ));
            }
            if request.trust.require_response_model
                && self
                    .provenance
                    .response_model
                    .as_deref()
                    .is_none_or(str::is_empty)
            {
                errors.push(ExchangeValidationError::new(
                    "missing_provenance",
                    "provenance.response_model",
                    "the request requires actual response-model identity",
                ));
            }
        }
        if let Some(usage) = &self.usage {
            if usage.latency_ms == 0 {
                invalid_positive(&mut errors, "usage.latency_ms");
            }
            if usage
                .cost_usd
                .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
            {
                errors.push(ExchangeValidationError::new(
                    "invalid_usage",
                    "usage.cost_usd",
                    "cost_usd must be finite and non-negative",
                ));
            }
        }
        let started_at = validate_timestamp(
            &mut errors,
            "provenance.started_at",
            &self.provenance.started_at,
        );
        let finished_at = validate_timestamp(
            &mut errors,
            "provenance.finished_at",
            &self.provenance.finished_at,
        );
        if started_at
            .zip(finished_at)
            .is_some_and(|(start, finish)| finish < start)
        {
            errors.push(ExchangeValidationError::new(
                "invalid_provenance",
                "provenance.finished_at",
                "finished_at must not precede started_at",
            ));
        }
        if let Some(error) = &self.error {
            validate_nonempty(&mut errors, "error.code", &error.code);
            validate_nonempty(&mut errors, "error.message", &error.message);
        }
        finish(errors)
    }
}

fn deserialize_request_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, RUNNER_EXCHANGE_REQUEST_SCHEMA)
}

fn deserialize_result_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, RUNNER_EXCHANGE_RESULT_SCHEMA)
}

fn validate_adapter(
    errors: &mut Vec<ExchangeValidationError>,
    field: &str,
    adapter: &AdapterIdentity,
) {
    validate_nonempty(errors, &format!("{field}.name"), &adapter.name);
    validate_nonempty(errors, &format!("{field}.version"), &adapter.version);
    validate_prefixed_digest(errors, &format!("{field}.digest"), &adapter.digest);
}

fn validate_candidate(
    errors: &mut Vec<ExchangeValidationError>,
    field: &str,
    candidate: &CandidateIdentity,
) {
    validate_nonempty(errors, &format!("{field}.harness"), &candidate.harness);
    validate_nonempty(
        errors,
        &format!("{field}.harness_version"),
        &candidate.harness_version,
    );
    if matches!(candidate.kind, CandidateKind::Model) && candidate.model.is_none() {
        errors.push(ExchangeValidationError::new(
            "candidate_identity",
            &format!("{field}.model"),
            "a model candidate requires model identity",
        ));
    }
    if matches!(candidate.kind, CandidateKind::Deterministic)
        && (candidate.model.is_some()
            || candidate.prompt_hash.is_some()
            || candidate.reasoning_effort.is_some())
    {
        errors.push(ExchangeValidationError::new(
            "candidate_identity",
            field,
            "a deterministic candidate cannot claim model, prompt, or reasoning identity",
        ));
    }
    if let Some(model) = &candidate.model {
        validate_nonempty(errors, &format!("{field}.model.provider"), &model.provider);
        validate_nonempty(errors, &format!("{field}.model.name"), &model.name);
    }
    if let Some(hash) = &candidate.prompt_hash {
        validate_prefixed_digest(errors, &format!("{field}.prompt_hash"), hash);
    }
    if let Some(hash) = &candidate.toolset_hash {
        validate_prefixed_digest(errors, &format!("{field}.toolset_hash"), hash);
    }
}

fn validate_nonempty(errors: &mut Vec<ExchangeValidationError>, field: &str, value: &str) {
    if value.trim().is_empty() {
        errors.push(ExchangeValidationError::new(
            "missing_identity",
            field,
            "value must not be blank",
        ));
    }
}

fn validate_schema(
    errors: &mut Vec<ExchangeValidationError>,
    field: &str,
    actual: &str,
    expected: &str,
) {
    if actual != expected {
        errors.push(ExchangeValidationError::new(
            "schema_mismatch",
            field,
            format!("expected schema {expected:?}, got {actual:?}"),
        ));
    }
}

fn validate_unique_nonempty(
    errors: &mut Vec<ExchangeValidationError>,
    field: &str,
    values: &[String],
) {
    let mut seen = BTreeSet::new();
    for (index, value) in values.iter().enumerate() {
        validate_nonempty(errors, &format!("{field}[{index}]"), value);
        if !seen.insert(value) {
            errors.push(ExchangeValidationError::new(
                "duplicate_value",
                &format!("{field}[{index}]"),
                "values must be unique",
            ));
        }
    }
}

fn validate_relative(errors: &mut Vec<ExchangeValidationError>, field: &str, value: &str) {
    if !is_portable_relative(value) {
        errors.push(ExchangeValidationError::new(
            "unsafe_path",
            field,
            "path must be a portable relative path without parent traversal",
        ));
    }
}

fn is_portable_relative(value: &str) -> bool {
    let path = Path::new(value);
    !value.trim().is_empty()
        && !value.contains('\\')
        && !value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn authority_covers(path: &str, roots: &[String]) -> bool {
    roots.iter().any(|root| {
        is_portable_relative(root) && (root == "." || Path::new(path).starts_with(Path::new(root)))
    })
}

fn validate_prefixed_digest(errors: &mut Vec<ExchangeValidationError>, field: &str, value: &str) {
    let valid = value
        .strip_prefix("sha256:")
        .is_some_and(|hex| is_hex_len(hex, 64));
    if !valid {
        errors.push(ExchangeValidationError::new(
            "invalid_digest",
            field,
            "digest must be sha256: followed by 64 lowercase hexadecimal characters",
        ));
    }
}

fn is_hex_len(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_credential_reference(value: &str) -> bool {
    value.strip_prefix("ref:").is_some_and(|path| {
        !path.is_empty()
            && path.len() <= 200
            && path != "."
            && is_portable_relative(path)
            && path.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-')
            })
    })
}

fn validate_timestamp(
    errors: &mut Vec<ExchangeValidationError>,
    field: &str,
    value: &str,
) -> Option<OffsetDateTime> {
    match OffsetDateTime::parse(value, &Rfc3339) {
        Ok(timestamp) => Some(timestamp),
        Err(_) => {
            errors.push(ExchangeValidationError::new(
                "invalid_provenance",
                field,
                "timestamp must be RFC 3339",
            ));
            None
        }
    }
}

fn invalid_positive(errors: &mut Vec<ExchangeValidationError>, field: &str) {
    errors.push(ExchangeValidationError::new(
        "invalid_limit",
        field,
        "value must be greater than zero",
    ));
}

fn identity_mismatch(errors: &mut Vec<ExchangeValidationError>, field: &str) {
    errors.push(ExchangeValidationError::new(
        "identity_mismatch",
        field,
        "result identity must exactly match the request",
    ));
}

fn finish(errors: Vec<ExchangeValidationError>) -> Result<(), Vec<ExchangeValidationError>> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
