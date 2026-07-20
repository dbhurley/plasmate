use std::path::PathBuf;

#[test]
fn v8_compatibility_manifest_cannot_drift_from_locked_build() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("compatibility/v8.json")).expect("read V8 manifest"),
    )
    .expect("parse V8 manifest");
    assert_eq!(
        manifest["schema"], "plasmate.v8-compatibility.v1",
        "compatibility schema changed without updating the checker"
    );
    let selected = manifest["binding"]["selected"]
        .as_str()
        .expect("selected V8 version");
    assert_eq!(manifest["binding"]["highest_api_compatible"], selected);
    assert_eq!(manifest["upgrade_gap"]["severity"], "critical");

    let cargo: toml::Value = std::fs::read_to_string(root.join("Cargo.toml"))
        .expect("read Cargo.toml")
        .parse()
        .expect("parse Cargo.toml");
    let expected_requirement = format!("={selected}");
    assert_eq!(
        cargo["dependencies"]["v8"].as_str(),
        Some(expected_requirement.as_str())
    );
    let icu_crate = manifest["icu_data"]["crate"]
        .as_str()
        .expect("ICU data crate");
    let icu_selected = manifest["icu_data"]["selected"]
        .as_str()
        .expect("selected ICU data version");
    assert_eq!(manifest["icu_data"]["icu_major"], 74);
    assert_eq!(
        manifest["icu_data"]["registration_api"],
        "set_common_data_74"
    );
    let expected_icu_requirement = format!("={icu_selected}");
    assert_eq!(
        cargo["dependencies"][icu_crate].as_str(),
        Some(expected_icu_requirement.as_str())
    );
    let project_rust = manifest["project"]["minimum_rust"]
        .as_str()
        .expect("project MSRV")
        .trim_end_matches(".0");
    assert_eq!(
        cargo["package"]["rust-version"].as_str(),
        Some(project_rust)
    );

    let lock: toml::Value = std::fs::read_to_string(root.join("Cargo.lock"))
        .expect("read Cargo.lock")
        .parse()
        .expect("parse Cargo.lock");
    let locked: Vec<_> = lock["package"]
        .as_array()
        .expect("Cargo.lock package list")
        .iter()
        .filter(|package| package["name"].as_str() == Some("v8"))
        .filter_map(|package| package["version"].as_str())
        .collect();
    assert_eq!(locked, vec![selected]);
    let locked_icu: Vec<_> = lock["package"]
        .as_array()
        .expect("Cargo.lock package list")
        .iter()
        .filter(|package| package["name"].as_str() == Some(icu_crate))
        .filter_map(|package| package["version"].as_str())
        .collect();
    assert_eq!(locked_icu, vec![icu_selected]);
}

#[test]
fn required_ci_enforces_the_v8_compatibility_contract() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("read required CI workflow");
    let action_manifest = ci
        .split_once("  action-manifest:\n")
        .map(|(_, job)| job)
        .expect("action-manifest job must exist");
    assert!(
        action_manifest.contains(
            "      - name: V8 compatibility contract\n        run: python3 scripts/check-v8-compatibility.py"
        ),
        "the required action-manifest job must enforce V8 compatibility drift"
    );
}
