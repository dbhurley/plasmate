//! Dry-run validation for independently versioned release artifacts.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub artifacts: Vec<ReleaseArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseArtifact {
    pub id: String,
    pub package: String,
    pub version: String,
    pub publications: Vec<Publication>,
    pub sources: Vec<VersionSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Publication {
    CratesIo,
    GithubRelease,
    Ghcr,
    Npm,
    Pypi,
    McpRegistry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionSource {
    pub path: String,
    pub kind: SourceKind,
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    JsonPointer,
    TomlKey,
    /// Selects the version from a `[[package]]` table with the given package name.
    TomlPackage,
    /// The first capture group from a regular expression over a UTF-8 source file.
    TextRegex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub schema_version: u32,
    pub manifest: String,
    pub valid: bool,
    pub artifacts_total: usize,
    pub sources_total: usize,
    pub sources_valid: usize,
    pub errors: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub artifact: String,
    pub path: String,
    pub expected: String,
    pub actual: Option<String>,
    pub message: String,
}

pub fn validate(
    repository_root: &Path,
    manifest_path: impl AsRef<Path>,
) -> Result<ValidationReport, Box<dyn std::error::Error>> {
    let manifest_path = resolve(repository_root, manifest_path.as_ref());
    let manifest: ReleaseManifest = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported release manifest schema {}; expected 1",
            manifest.schema_version
        )
        .into());
    }

    let mut errors = Vec::new();
    let mut artifact_ids = HashSet::new();
    let mut sources_total: usize = 0;
    for artifact in &manifest.artifacts {
        if !artifact_ids.insert(artifact.id.as_str()) {
            errors.push(ValidationIssue {
                artifact: artifact.id.clone(),
                path: String::new(),
                expected: artifact.version.clone(),
                actual: None,
                message: "artifact id is duplicated".to_string(),
            });
        }
        if artifact.publications.is_empty() {
            errors.push(ValidationIssue {
                artifact: artifact.id.clone(),
                path: String::new(),
                expected: artifact.version.clone(),
                actual: None,
                message: "artifact has no publication destination".to_string(),
            });
        }
        if artifact.sources.is_empty() {
            errors.push(ValidationIssue {
                artifact: artifact.id.clone(),
                path: String::new(),
                expected: artifact.version.clone(),
                actual: None,
                message: "artifact has no checked-in version source".to_string(),
            });
        }
        for source in &artifact.sources {
            sources_total += 1;
            match read_version(repository_root, source) {
                Ok(actual) if actual == artifact.version => {}
                Ok(actual) => errors.push(ValidationIssue {
                    artifact: artifact.id.clone(),
                    path: source.path.clone(),
                    expected: artifact.version.clone(),
                    actual: Some(actual),
                    message: "checked-in version differs from release manifest".to_string(),
                }),
                Err(message) => errors.push(ValidationIssue {
                    artifact: artifact.id.clone(),
                    path: source.path.clone(),
                    expected: artifact.version.clone(),
                    actual: None,
                    message,
                }),
            }
        }
    }

    let sources_valid =
        sources_total.saturating_sub(errors.iter().filter(|issue| !issue.path.is_empty()).count());
    Ok(ValidationReport {
        schema_version: 1,
        manifest: manifest_path.display().to_string(),
        valid: errors.is_empty(),
        artifacts_total: manifest.artifacts.len(),
        sources_total,
        sources_valid,
        errors,
    })
}

fn read_version(repository_root: &Path, source: &VersionSource) -> Result<String, String> {
    let path = secure_source_path(repository_root, Path::new(&source.path))?;
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    match source.kind {
        SourceKind::JsonPointer => {
            let value: serde_json::Value =
                serde_json::from_str(&content).map_err(|error| format!("invalid JSON: {error}"))?;
            value
                .pointer(&source.selector)
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("JSON pointer {} is not a string", source.selector))
        }
        SourceKind::TomlKey => {
            let value: toml::Value =
                toml::from_str(&content).map_err(|error| format!("invalid TOML: {error}"))?;
            dotted_toml_value(&value, &source.selector)
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("TOML key {} is not a string", source.selector))
        }
        SourceKind::TomlPackage => {
            let value: toml::Value =
                toml::from_str(&content).map_err(|error| format!("invalid TOML: {error}"))?;
            value
                .get("package")
                .and_then(|packages| packages.as_array())
                .and_then(|packages| {
                    packages.iter().find(|package| {
                        package.get("name").and_then(|name| name.as_str())
                            == Some(source.selector.as_str())
                    })
                })
                .and_then(|package| package.get("version"))
                .and_then(|version| version.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("TOML package {} has no string version", source.selector))
        }
        SourceKind::TextRegex => {
            let pattern = regex::Regex::new(&source.selector)
                .map_err(|error| format!("invalid version regex: {error}"))?;
            pattern
                .captures(&content)
                .and_then(|captures| captures.get(1))
                .map(|capture| capture.as_str().to_string())
                .ok_or_else(|| "version regex did not produce capture group 1".to_string())
        }
    }
}

fn secure_source_path(repository_root: &Path, path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Err("release source path must be repository-relative, not absolute".to_string());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(
                    "release source path must not contain parent-directory traversal".to_string(),
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("release source path must be repository-relative".to_string());
            }
        }
    }

    let canonical_root = repository_root
        .canonicalize()
        .map_err(|error| format!("cannot resolve repository root: {error}"))?;
    let candidate = canonical_root.join(path);
    let canonical_candidate = candidate
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", candidate.display()))?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err("release source path resolves outside the repository".to_string());
    }
    Ok(canonical_candidate)
}

fn dotted_toml_value<'a>(value: &'a toml::Value, key: &str) -> Option<&'a toml::Value> {
    key.split('.')
        .try_fold(value, |current, component| current.get(component))
}

fn resolve(repository_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository_root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_release_manifest_is_coherent() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let report = validate(root, "release-manifest.json").expect("validate release manifest");
        assert!(report.valid, "{:?}", report.errors);
        assert_eq!(report.sources_total, report.sources_valid);
    }

    #[test]
    fn detects_version_drift_without_publishing() {
        let directory = tempfile::tempdir().expect("temporary repository");
        fs::write(
            directory.path().join("package.json"),
            r#"{"name":"example","version":"1.1.0"}"#,
        )
        .expect("write package");
        fs::write(
            directory.path().join("release-manifest.json"),
            r#"{
              "schema_version": 1,
              "artifacts": [{
                "id": "example",
                "package": "example",
                "version": "1.2.0",
                "publications": ["npm"],
                "sources": [{
                  "path": "package.json",
                  "kind": "json_pointer",
                  "selector": "/version"
                }]
              }]
            }"#,
        )
        .expect("write manifest");

        let report = validate(directory.path(), "release-manifest.json").expect("validate");
        assert!(!report.valid);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].expected, "1.2.0");
        assert_eq!(report.errors[0].actual.as_deref(), Some("1.1.0"));
    }

    fn write_manifest_with_source(directory: &Path, source_path: &str) {
        let manifest = serde_json::json!({
            "schema_version": 1,
            "artifacts": [{
                "id": "example",
                "package": "example",
                "version": "1.2.0",
                "publications": ["npm"],
                "sources": [{
                    "path": source_path,
                    "kind": "json_pointer",
                    "selector": "/version"
                }]
            }]
        });
        fs::write(
            directory.join("release-manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
    }

    #[test]
    fn rejects_absolute_release_source_paths() {
        let directory = tempfile::tempdir().expect("temporary repository");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        write_manifest_with_source(
            directory.path(),
            outside.path().to_str().expect("UTF-8 path"),
        );

        let report = validate(directory.path(), "release-manifest.json").expect("validate");
        assert!(!report.valid);
        assert!(report.errors[0].message.contains("not absolute"));
    }

    #[test]
    fn rejects_parent_directory_traversal() {
        let directory = tempfile::tempdir().expect("temporary repository");
        write_manifest_with_source(directory.path(), "../outside.json");

        let report = validate(directory.path(), "release-manifest.json").expect("validate");
        assert!(!report.valid);
        assert!(report.errors[0]
            .message
            .contains("parent-directory traversal"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_that_resolve_outside_repository() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary repository");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        symlink(outside.path(), directory.path().join("version.json")).expect("create symlink");
        write_manifest_with_source(directory.path(), "version.json");

        let report = validate(directory.path(), "release-manifest.json").expect("validate");
        assert!(!report.valid);
        assert!(report.errors[0].message.contains("outside the repository"));
    }
}
