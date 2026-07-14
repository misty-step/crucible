use crucible_core::{
    CandidateKind, ExchangeError, ExchangeStatus, RunnerExchangeRequest, RunnerExchangeResult,
};

fn harbor_request() -> RunnerExchangeRequest {
    serde_json::from_str(include_str!("fixtures/runner-exchange/harbor-request.json"))
        .expect("Harbor-like request fixture parses")
}

fn harbor_result() -> RunnerExchangeResult {
    serde_json::from_str(include_str!("fixtures/runner-exchange/harbor-result.json"))
        .expect("Harbor-like result fixture parses")
}

#[test]
fn harbor_like_exchange_conforms_without_a_runner_kind() {
    let request = harbor_request();
    let result = harbor_result();
    request.validate().expect("request conforms");
    result
        .validate_against(&request)
        .expect("result conforms to request");
    assert_eq!(request.candidate.kind, CandidateKind::Agent);
    assert_eq!(result.status, ExchangeStatus::Success);
}

#[test]
fn second_generic_adapter_uses_the_same_contract() {
    let request: RunnerExchangeRequest = serde_json::from_str(include_str!(
        "fixtures/runner-exchange/generic-request.json"
    ))
    .expect("generic request parses");
    let result: RunnerExchangeResult =
        serde_json::from_str(include_str!("fixtures/runner-exchange/generic-result.json"))
            .expect("generic result parses");
    request.validate().expect("request conforms");
    result
        .validate_against(&request)
        .expect("result conforms to request");
}

#[test]
fn additive_unknown_fields_survive_a_round_trip() {
    let request = harbor_request();
    assert!(request.extra.contains_key("future_hint"));
    let wire = serde_json::to_value(request).expect("serialize request");
    assert_eq!(wire["future_hint"]["safe_to_ignore"], true);
    assert_eq!(
        wire["adapter_payload"]["agent_import_path"],
        "harbor.agents.codex:Codex"
    );
}

#[test]
fn additive_result_fields_and_adapter_payload_survive_a_round_trip() {
    let mut wire: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/runner-exchange/generic-result.json"))
            .expect("fixture is JSON");
    wire["future_receipt"] = serde_json::json!({"revision": 2});
    let result: RunnerExchangeResult = serde_json::from_value(wire).expect("result parses");
    let round_trip = serde_json::to_value(result).expect("result serializes");
    assert_eq!(round_trip["future_receipt"]["revision"], 2);
    assert_eq!(round_trip["adapter_payload"]["request_id"], "example-123");
}

#[test]
fn major_schema_mismatch_is_refused_at_parse_time() {
    let wire = include_str!("fixtures/runner-exchange/harbor-request.json").replace(
        "crucible.runner_exchange_request.v1",
        "crucible.runner_exchange_request.v2",
    );
    let err = serde_json::from_str::<RunnerExchangeRequest>(&wire)
        .expect_err("v2 must not be read as v1")
        .to_string();
    assert!(err.contains("v2") && err.contains("v1"), "{err}");
}

#[test]
fn unsafe_evidence_path_is_rejected() {
    let request = harbor_request();
    let mut result = harbor_result();
    result.evidence[0].path = "../escaped.json".to_string();
    let errors = result
        .validate_against(&request)
        .expect_err("parent traversal must fail");
    assert!(errors.iter().any(|error| error.code == "unsafe_path"));
}

#[test]
fn candidate_identity_cannot_contradict_its_kind() {
    let mut request = harbor_request();
    request.candidate.kind = CandidateKind::Deterministic;
    let errors = request
        .validate()
        .expect_err("deterministic + model must fail");
    assert!(errors
        .iter()
        .any(|error| error.code == "candidate_identity"));
}

#[test]
fn success_requires_output_and_required_evidence() {
    let request = harbor_request();
    let mut result = harbor_result();
    result.output = None;
    result.evidence.retain(|item| item.kind != "transcript");
    let errors = result
        .validate_against(&request)
        .expect_err("incomplete success must fail");
    assert!(errors.iter().any(|error| error.code == "missing_output"));
    assert!(errors.iter().any(|error| error.code == "missing_evidence"));
}

#[test]
fn timeout_requires_a_structured_error_and_forbids_success_output() {
    let request = harbor_request();
    let mut result = harbor_result();
    result.status = ExchangeStatus::Timeout;
    result.error = None;
    let errors = result
        .validate_against(&request)
        .expect_err("timeout without error must fail");
    assert!(errors.iter().any(|error| error.code == "missing_error"));
    assert!(errors.iter().any(|error| error.code == "unexpected_output"));
}

#[test]
fn every_non_success_outcome_has_one_structured_wire_shape() {
    let request = harbor_request();
    for status in [
        ExchangeStatus::Refused,
        ExchangeStatus::Timeout,
        ExchangeStatus::MalformedOutput,
        ExchangeStatus::ExecutionError,
    ] {
        let mut result = harbor_result();
        result.status = status;
        result.output = None;
        result.evidence.clear();
        result.usage = None;
        result.error = Some(ExchangeError {
            code: "adapter_failure".to_string(),
            message: "bounded diagnostic".to_string(),
            retryable: matches!(status, ExchangeStatus::Timeout),
            detail: serde_json::json!({"exit_code": 7}),
        });
        result
            .validate_against(&request)
            .unwrap_or_else(|errors| panic!("{status:?} should conform: {errors:#?}"));
    }
}

#[test]
fn invalid_limits_and_usage_are_rejected() {
    let mut request = harbor_request();
    request.limits.timeout_ms = 0;
    let errors = request.validate().expect_err("zero timeout must fail");
    assert!(errors
        .iter()
        .any(|error| error.field == "limits.timeout_ms"));

    let request = harbor_request();
    let mut result = harbor_result();
    result.usage.as_mut().expect("usage").cost_usd = Some(f64::NAN);
    let errors = result
        .validate_against(&request)
        .expect_err("NaN cost must fail");
    assert!(errors.iter().any(|error| error.code == "invalid_usage"));
}

#[test]
fn request_authority_paths_are_confined_too() {
    let mut request = harbor_request();
    request.authority.filesystem_roots = vec!["/host".to_string()];
    let errors = request
        .validate()
        .expect_err("absolute authority root must fail");
    assert!(errors.iter().any(|error| error.code == "unsafe_path"));
}

#[test]
fn every_input_path_requires_declared_filesystem_authority() {
    let mut request = harbor_request();
    request.authority.filesystem_roots = vec!["workspace".to_string()];
    let errors = request
        .validate()
        .expect_err("sibling instruction and context paths must be granted");
    assert!(errors.iter().any(|error| error.code == "ungranted_path"));
}

#[test]
fn drive_shaped_paths_are_not_portable_relative_paths() {
    let mut request = harbor_request();
    request.input.workspace = "C:/host/path".to_string();
    let errors = request.validate().expect_err("drive-shaped path must fail");
    assert!(errors
        .iter()
        .any(|error| error.code == "unsafe_path" && error.field == "input.workspace"));
}

#[test]
fn credential_reference_paths_are_confined() {
    for credential_ref in ["ref:../../secret", "ref:/secret", "ref:C:/secret"] {
        let mut request = harbor_request();
        request.authority.credential_refs = vec![credential_ref.to_string()];
        let errors = request
            .validate()
            .expect_err("credential broker traversal must fail");
        assert!(errors
            .iter()
            .any(|error| error.code == "invalid_credential_reference"));
    }
}

#[test]
fn programmatic_schema_mismatch_and_invalid_timestamps_fail_validation() {
    let request = harbor_request();
    let mut bad_request = request.clone();
    bad_request.schema_version = "crucible.runner_exchange_request.v2".to_string();
    assert!(bad_request
        .validate()
        .expect_err("programmatic schema mismatch must fail")
        .iter()
        .any(|error| error.code == "schema_mismatch"));

    let mut result = harbor_result();
    result.provenance.started_at = "not-a-timestamp".to_string();
    result.provenance.finished_at = "2026-07-13T00:00:00Z".to_string();
    assert!(result
        .validate_against(&request)
        .expect_err("malformed timestamp must fail")
        .iter()
        .any(|error| error.field == "provenance.started_at"));

    let mut reversed = harbor_result();
    reversed.provenance.started_at = "2026-07-13T00:00:01Z".to_string();
    reversed.provenance.finished_at = "2026-07-13T00:00:00Z".to_string();
    assert!(reversed
        .validate_against(&request)
        .expect_err("reversed timestamps must fail")
        .iter()
        .any(|error| error.field == "provenance.finished_at"));
}

#[test]
fn credential_values_and_url_shaped_hosts_are_not_authority_references() {
    let mut request = harbor_request();
    request.authority.credential_refs = vec!["api_key=embedded-value".to_string()];
    request.authority.network = crucible_core::NetworkAuthority::Allowlist;
    request.authority.allowed_hosts = vec!["https://user@api.example.test".to_string()];
    let errors = request
        .validate()
        .expect_err("embedded authority must fail");
    assert!(errors
        .iter()
        .any(|error| error.code == "invalid_credential_reference"));
    assert!(errors.iter().any(|error| error.code == "invalid_host"));
}

#[test]
fn success_honors_cost_and_response_model_trust_requirements() {
    let request = harbor_request();
    let mut result = harbor_result();
    result.usage.as_mut().expect("usage").cost_usd = None;
    result.provenance.response_model = None;
    let errors = result
        .validate_against(&request)
        .expect_err("required cost and response model must fail closed");
    assert!(errors.iter().any(|error| error.field == "usage.cost_usd"));
    assert!(errors
        .iter()
        .any(|error| error.field == "provenance.response_model"));
}
