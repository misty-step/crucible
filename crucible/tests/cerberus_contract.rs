use serde_json::Value;

const CERBERUS_PRODUCER_MANIFEST_SCHEMA_VERSION: &str = "cerberus.crucible_producer_manifest.v1";

#[test]
fn cerberus_producer_manifest_fixture_matches_pinned_contract() {
    let schema: Value = serde_json::from_str(include_str!(
        "fixtures/contracts/cerberus.crucible_producer_manifest.v1.schema.json"
    ))
    .unwrap();
    assert_eq!(
        schema["properties"]["schema_version"]["const"],
        CERBERUS_PRODUCER_MANIFEST_SCHEMA_VERSION
    );
    assert_schema_requires(
        &schema,
        &[
            "schema_version",
            "consumer",
            "request",
            "artifact",
            "receipt_bundle",
            "grader_input",
            "validation",
            "boundary",
        ],
    );

    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/contracts/cerberus.crucible_producer_manifest.v1.valid.json"
    ))
    .unwrap();
    assert_cerberus_producer_manifest(&fixture).unwrap();
}

#[test]
fn cerberus_producer_manifest_contract_fails_on_field_rename() {
    let mut fixture: Value = serde_json::from_str(include_str!(
        "fixtures/contracts/cerberus.crucible_producer_manifest.v1.valid.json"
    ))
    .unwrap();
    let artifact = fixture["artifact"].as_object_mut().unwrap();
    let artifact_uri = artifact.remove("artifact_uri").unwrap();
    artifact.insert("uri".to_string(), artifact_uri);

    let error = assert_cerberus_producer_manifest(&fixture).unwrap_err();
    assert!(error.contains("artifact.artifact_uri"), "{error}");
}

#[test]
fn cerberus_producer_manifest_rejects_unknown_major_version() {
    let mut fixture: Value = serde_json::from_str(include_str!(
        "fixtures/contracts/cerberus.crucible_producer_manifest.v1.valid.json"
    ))
    .unwrap();
    fixture["schema_version"] = Value::String("cerberus.crucible_producer_manifest.v2".to_string());

    let error = assert_cerberus_producer_manifest(&fixture).unwrap_err();
    assert!(
        error.contains("unsupported schema_version cerberus.crucible_producer_manifest.v2"),
        "{error}"
    );
}

fn assert_schema_requires(schema: &Value, fields: &[&str]) {
    let required = schema["required"].as_array().unwrap();
    for field in fields {
        assert!(
            required.iter().any(|required| required == field),
            "schema required list missing {field}: {schema}"
        );
    }
}

fn assert_cerberus_producer_manifest(value: &Value) -> Result<(), String> {
    let schema_version = required_string(value, "schema_version")?;
    if schema_version != CERBERUS_PRODUCER_MANIFEST_SCHEMA_VERSION {
        return Err(format!("unsupported schema_version {schema_version}"));
    }
    expect_string(value, "consumer", "consumer", "crucible")?;

    let request = required_object(value, "request")?;
    required_string_at(request, "request_id", "request.request_id")?;
    required_string_at(request, "request_digest", "request.request_digest")?;

    let artifact = required_object(value, "artifact")?;
    required_string_at(artifact, "artifact_id", "artifact.artifact_id")?;
    required_string_at(artifact, "artifact_uri", "artifact.artifact_uri")?;
    required_string_at(artifact, "artifact_digest", "artifact.artifact_digest")?;
    expect_string(
        artifact,
        "schema_version",
        "artifact.schema_version",
        "cerberus.review_artifact.v1",
    )?;
    required_u64_at(artifact, "finding_count", "artifact.finding_count")?;
    required_u64_at(artifact, "comment_count", "artifact.comment_count")?;
    required_string_at(artifact, "capability_tier", "artifact.capability_tier")?;
    required_object_at(
        artifact,
        "context_capabilities",
        "artifact.context_capabilities",
    )?;

    let receipt = required_object(value, "receipt_bundle")?;
    expect_string(
        receipt,
        "schema_version",
        "receipt_bundle.schema_version",
        "cerberus.review_receipt_bundle.v1",
    )?;
    required_string_at(
        receipt,
        "receipt_bundle_uri",
        "receipt_bundle.receipt_bundle_uri",
    )?;
    required_string_at(
        receipt,
        "receipt_bundle_digest",
        "receipt_bundle.receipt_bundle_digest",
    )?;

    let grader_input = required_object(value, "grader_input")?;
    expect_string(
        grader_input,
        "format",
        "grader_input.format",
        "cerberus.review_artifact.v1",
    )?;
    required_string_at(grader_input, "artifact_uri", "grader_input.artifact_uri")?;
    expect_string(
        grader_input,
        "findings_path",
        "grader_input.findings_path",
        "findings",
    )?;
    expect_string(
        grader_input,
        "finding_id_path",
        "grader_input.finding_id_path",
        "findings[].id",
    )?;

    let validation = required_object(value, "validation")?;
    required_string_at(validation, "status", "validation.status")?;
    required_bool_at(
        validation,
        "trusted_for_grading",
        "validation.trusted_for_grading",
    )?;

    let boundary = required_object(value, "boundary")?;
    expect_string(
        boundary,
        "scorer_owner",
        "boundary.scorer_owner",
        "crucible",
    )?;
    required_bool_at(boundary, "includes_score", "boundary.includes_score")?;
    required_string_at(boundary, "note", "boundary.note")?;
    Ok(())
}

fn required_object<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    required_object_at(value, key, key)
}

fn required_object_at<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a Value, String> {
    match value.get(key) {
        Some(value @ Value::Object(_)) => Ok(value),
        _ => Err(format!("missing object {label}")),
    }
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    required_string_at(value, key, key)
}

fn required_string_at<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a str, String> {
    match value.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(format!("missing string {label}")),
    }
}

fn required_u64_at(value: &Value, key: &str, label: &str) -> Result<u64, String> {
    match value.get(key).and_then(Value::as_u64) {
        Some(value) => Ok(value),
        _ => Err(format!("missing integer {label}")),
    }
}

fn required_bool_at(value: &Value, key: &str, label: &str) -> Result<bool, String> {
    match value.get(key).and_then(Value::as_bool) {
        Some(value) => Ok(value),
        _ => Err(format!("missing bool {label}")),
    }
}

fn expect_string(value: &Value, key: &str, label: &str, expected: &str) -> Result<(), String> {
    let actual = required_string_at(value, key, label)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!("expected {label}={expected}, got {actual}"))
    }
}
