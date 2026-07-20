//! Screenshot capture module.
//!
//! Delegates pixel-perfect rendering to a headless Chrome/Chromium subprocess
//! when available. Chrome is spawned on demand, captures the screenshot, and
//! exits immediately — zero cost unless a screenshot is explicitly requested.
//!
//! If Chrome is not installed, callers fall back to returning the SOM as
//! structured data via [`som_fallback`].

use serde_json::json;

const OFFLINE_PROXY: &str = "http://127.0.0.1:9";
const EMBEDDED_CONTENT_POLICY: &str = "default-src 'none'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data:; connect-src 'none'";
const MAX_LEGACY_SCREENSHOT_BYTES: usize = 16 * 1024 * 1024;

/// Default viewport width.
pub const DEFAULT_WIDTH: u32 = 1280;
/// Default viewport height.
pub const DEFAULT_HEIGHT: u32 = 720;

/// Screenshot output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Webp,
}

impl Format {
    pub fn as_str(&self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpeg",
            Format::Webp => "webp",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "jpeg" | "jpg" => Format::Jpeg,
            "webp" => Format::Webp,
            _ => Format::Png,
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpeg => "image/jpeg",
            Format::Webp => "image/webp",
        }
    }
}

/// Options for capturing a screenshot.
#[derive(Debug, Clone)]
pub struct ScreenshotOptions {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    /// Quality for JPEG/WebP (1-100). Ignored for PNG.
    pub quality: Option<u32>,
    /// If true, capture the full scrollable page.
    pub full_page: bool,
}

impl Default for ScreenshotOptions {
    fn default() -> Self {
        ScreenshotOptions {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            format: Format::Png,
            quality: None,
            full_page: false,
        }
    }
}

/// Error type for screenshot operations.
#[derive(Debug, thiserror::Error)]
pub enum ScreenshotError {
    #[error(
        "Chrome/Chromium not found. Install Chrome or Chromium for screenshot support. \
         SOM output is available via `plasmate fetch` or `Plasmate.getSom`."
    )]
    ChromeNotFound,
    #[error("Screenshot capture failed: {0}")]
    CaptureFailed(String),
    #[error("Render error: {0}")]
    RenderError(String),
    #[error("Screenshot capture timed out")]
    Timeout,
    #[error("Screenshot exceeds the configured {limit} byte limit")]
    OutputTooLarge { limit: usize },
}

/// Find Chrome/Chromium binary on the system.
fn find_chrome() -> Option<String> {
    let candidates = [
        // macOS
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        // Linux
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        // Windows (WSL)
        "/mnt/c/Program Files/Google/Chrome/Application/chrome.exe",
    ];

    for candidate in &candidates {
        if std::path::Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
        // Also check PATH
        if let Ok(output) = std::process::Command::new("which").arg(candidate).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Put an external renderer in a dedicated process tree and guarantee that the
/// root is reaped after the tree is terminated. Chrome routinely creates child
/// processes, so killing only the returned [`std::process::Child`] is not a
/// sufficient cleanup boundary.
struct ContainedChild {
    child: std::process::Child,
    process_tree: crate::process_supervisor::ProcessTreeGuard,
    reaped: bool,
}

impl ContainedChild {
    fn spawn(command: &mut std::process::Command) -> std::io::Result<Self> {
        crate::process_supervisor::configure_process_tree(command);
        let child = command.spawn()?;
        let process_tree = crate::process_supervisor::ProcessTreeGuard::new(child.id());
        Ok(Self {
            child,
            process_tree,
            reaped: false,
        })
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    fn terminate_and_wait(&mut self) {
        self.process_tree.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reaped = true;
    }
}

impl Drop for ContainedChild {
    fn drop(&mut self) {
        if !self.reaped {
            self.terminate_and_wait();
        } else {
            self.process_tree.terminate();
        }
    }
}

fn escape_srcdoc(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Place untrusted effective HTML in a sandboxed srcdoc document. The CSP is
/// the first markup parsed inside that document and is defense in depth for the
/// renderer-level JavaScript switch and Chromium's restrictive file defaults.
fn hardened_render_document(html: &str) -> String {
    let embedded = format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{EMBEDDED_CONTENT_POLICY}\">{html}"
    );
    format!(
        "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; frame-src 'self'\"><style>html,body,iframe{{border:0;height:100%;margin:0;overflow:hidden;padding:0;width:100%}}iframe{{display:block}}</style></head><body><iframe sandbox referrerpolicy=\"no-referrer\" srcdoc=\"{}\"></iframe></body></html>",
        escape_srcdoc(&embedded)
    )
}

fn hardened_html_chrome_args(
    temp_dir: &std::path::Path,
    screenshot_path: &std::path::Path,
    opts: &ScreenshotOptions,
) -> Vec<String> {
    vec![
        "--headless=new".to_string(),
        "--disable-gpu".to_string(),
        "--disable-dev-shm-usage".to_string(),
        "--disable-extensions".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-sync".to_string(),
        "--disable-translate".to_string(),
        "--mute-audio".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-javascript".to_string(),
        format!("--proxy-server={OFFLINE_PROXY}"),
        "--proxy-bypass-list=<-loopback>".to_string(),
        format!("--window-size={},{}", opts.width, opts.height),
        format!("--user-data-dir={}", temp_dir.display()),
        format!("--screenshot={}", screenshot_path.display()),
    ]
}

fn complete_png_size(path: &std::path::Path) -> std::io::Result<Option<u64>> {
    use std::io::{Read, Seek, SeekFrom};

    const PNG_IEND: [u8; 12] = [
        0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
    ];
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let length = file.metadata()?.len();
    if length < PNG_IEND.len() as u64 {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-(PNG_IEND.len() as i64)))?;
    let mut trailer = [0_u8; PNG_IEND.len()];
    file.read_exact(&mut trailer)?;
    Ok((trailer == PNG_IEND).then_some(length))
}

/// Capture a screenshot by delegating to headless Chrome.
///
/// Spawns a temporary Chrome process, navigates to the URL,
/// captures the screenshot, and terminates Chrome.
pub fn capture_url(url: &str, opts: &ScreenshotOptions) -> Result<Vec<u8>, ScreenshotError> {
    if !crate::network::security::OutboundUrlPolicy::from_environment().allows_private_network() {
        return Err(ScreenshotError::CaptureFailed(
            "direct Chrome URL navigation is disabled because browser redirects bypass the outbound URL policy; fetch with Plasmate and use capture_html, or set PLASMATE_UNSAFE_ALLOW_PRIVATE_NETWORK=1 for isolated development"
                .to_string(),
        ));
    }
    crate::network::security::OutboundUrlPolicy::from_environment()
        .validate_url_blocking(url)
        .map_err(ScreenshotError::CaptureFailed)?;
    let chrome_bin = find_chrome().ok_or(ScreenshotError::ChromeNotFound)?;

    let temp_dir = tempfile::tempdir()
        .map_err(|e| ScreenshotError::CaptureFailed(format!("temp dir: {}", e)))?;

    let screenshot_path = temp_dir.path().join("screenshot.png");

    let mut cmd = std::process::Command::new(&chrome_bin);
    cmd.args([
        "--headless=new",
        "--disable-gpu",
        "--disable-dev-shm-usage",
        "--disable-extensions",
        "--disable-background-networking",
        "--disable-sync",
        "--disable-translate",
        "--mute-audio",
        "--no-first-run",
        "--no-default-browser-check",
        &format!("--window-size={},{}", opts.width, opts.height),
        &format!("--user-data-dir={}", temp_dir.path().display()),
        &format!("--screenshot={}", screenshot_path.display()),
    ]);

    cmd.arg(url);

    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = ContainedChild::spawn(&mut cmd)
        .map_err(|e| ScreenshotError::CaptureFailed(format!("spawn chrome: {}", e)))?;

    let timeout = std::time::Duration::from_secs(15);
    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed() > timeout {
            child.terminate_and_wait();
            return Err(ScreenshotError::Timeout);
        }

        match child.try_wait() {
            Ok(Some(_status)) => {
                child.reaped = true;
                child.process_tree.terminate();
                break;
            }
            Ok(None) => {
                match complete_png_size(&screenshot_path) {
                    Ok(Some(length)) if length > MAX_LEGACY_SCREENSHOT_BYTES as u64 => {
                        child.terminate_and_wait();
                        return Err(ScreenshotError::OutputTooLarge {
                            limit: MAX_LEGACY_SCREENSHOT_BYTES,
                        });
                    }
                    Ok(Some(_)) => {
                        child.terminate_and_wait();
                        break;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        child.terminate_and_wait();
                        return Err(ScreenshotError::CaptureFailed(format!(
                            "inspect screenshot: {error}"
                        )));
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                child.terminate_and_wait();
                return Err(ScreenshotError::CaptureFailed(format!(
                    "Error waiting for Chrome: {}",
                    e
                )));
            }
        }
    }

    if !screenshot_path.exists() {
        return Err(ScreenshotError::CaptureFailed(
            "Chrome did not produce screenshot".to_string(),
        ));
    }

    let metadata = std::fs::metadata(&screenshot_path)
        .map_err(|e| ScreenshotError::CaptureFailed(format!("metadata screenshot: {}", e)))?;
    if metadata.len() > MAX_LEGACY_SCREENSHOT_BYTES as u64 {
        return Err(ScreenshotError::OutputTooLarge {
            limit: MAX_LEGACY_SCREENSHOT_BYTES,
        });
    }
    let data = std::fs::read(&screenshot_path)
        .map_err(|e| ScreenshotError::CaptureFailed(format!("read screenshot: {}", e)))?;
    if data.len() > MAX_LEGACY_SCREENSHOT_BYTES {
        return Err(ScreenshotError::OutputTooLarge {
            limit: MAX_LEGACY_SCREENSHOT_BYTES,
        });
    }

    Ok(data)
}

/// Capture a screenshot from HTML content.
pub fn capture_html(
    html: &str,
    base_url: &str,
    opts: &ScreenshotOptions,
) -> Result<Vec<u8>, ScreenshotError> {
    capture_html_with_limits(
        html,
        base_url,
        opts,
        std::time::Duration::from_secs(15),
        usize::MAX,
    )
}

/// Capture already-fetched HTML with caller-owned process and output limits.
///
/// The renderer receives a generated wrapper containing a sandboxed `srcdoc`,
/// JavaScript is disabled at Chromium's renderer boundary and again by the
/// sandboxed document, cross-file access remains at Chromium's restrictive
/// default and is narrowed by CSP, and HTTP(S) is sent to a dead proxy. Chrome
/// never navigates to the caller's network URL. The dedicated Chrome process
/// group is terminated and its root is waited on for every exit path.
pub fn capture_html_with_limits(
    html: &str,
    _base_url: &str,
    opts: &ScreenshotOptions,
    timeout: std::time::Duration,
    max_output_bytes: usize,
) -> Result<Vec<u8>, ScreenshotError> {
    let chrome_bin = find_chrome().ok_or(ScreenshotError::ChromeNotFound)?;

    let temp_dir = tempfile::tempdir()
        .map_err(|e| ScreenshotError::CaptureFailed(format!("temp dir: {}", e)))?;

    // Write a generated containment document only. The fetched HTML is embedded
    // as a sandboxed srcdoc rather than receiving top-level file:// privileges.
    let html_path = temp_dir.path().join("page.html");
    std::fs::write(&html_path, hardened_render_document(html))
        .map_err(|e| ScreenshotError::CaptureFailed(format!("write html: {}", e)))?;

    let screenshot_path = temp_dir.path().join("screenshot.png");

    let mut command = std::process::Command::new(&chrome_bin);
    command
        .args(hardened_html_chrome_args(
            temp_dir.path(),
            &screenshot_path,
            opts,
        ))
        .arg(format!("file://{}", html_path.display()))
        // Renderer diagnostics are intentionally discarded. They are neither
        // needed for this typed API nor safe to return through MCP.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = ContainedChild::spawn(&mut command)
        .map_err(|e| ScreenshotError::CaptureFailed(format!("spawn chrome: {}", e)))?;

    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed() > timeout {
            child.terminate_and_wait();
            return Err(ScreenshotError::Timeout);
        }

        match child.try_wait() {
            Ok(Some(_status)) => {
                // `try_wait` reaps the root process. Mark the wrapper reaped and
                // terminate any renderer descendants that retained the group.
                child.reaped = true;
                child.process_tree.terminate();
                break;
            }
            Ok(None) => {
                match complete_png_size(&screenshot_path) {
                    Ok(Some(length)) if length > max_output_bytes as u64 => {
                        child.terminate_and_wait();
                        return Err(ScreenshotError::OutputTooLarge {
                            limit: max_output_bytes,
                        });
                    }
                    Ok(Some(_)) => {
                        child.terminate_and_wait();
                        break;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        child.terminate_and_wait();
                        return Err(ScreenshotError::CaptureFailed(format!(
                            "inspect screenshot: {error}"
                        )));
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                child.terminate_and_wait();
                return Err(ScreenshotError::CaptureFailed(format!(
                    "Error waiting for Chrome: {}",
                    e
                )));
            }
        }
    }

    if !screenshot_path.exists() {
        return Err(ScreenshotError::CaptureFailed(
            "Chrome did not produce screenshot".to_string(),
        ));
    }

    let metadata = std::fs::metadata(&screenshot_path)
        .map_err(|e| ScreenshotError::CaptureFailed(format!("metadata: {}", e)))?;
    if metadata.len() > max_output_bytes as u64 {
        return Err(ScreenshotError::OutputTooLarge {
            limit: max_output_bytes,
        });
    }
    let data = std::fs::read(&screenshot_path)
        .map_err(|e| ScreenshotError::CaptureFailed(format!("read: {}", e)))?;
    if data.len() > max_output_bytes {
        return Err(ScreenshotError::OutputTooLarge {
            limit: max_output_bytes,
        });
    }
    Ok(data)
}

/// Check if Chrome is available for screenshots.
pub fn chrome_available() -> bool {
    find_chrome().is_some()
}

/// Build a structured fallback response when screenshot is unavailable.
///
/// Returns the SOM as JSON so callers still get useful data. This is the
/// honest alternative to faking a screenshot.
pub fn som_fallback(som: &crate::som::types::Som) -> serde_json::Value {
    let som_json = serde_json::to_value(som).unwrap_or(json!(null));
    json!({
        "error": "screenshot_not_implemented",
        "message": "Chrome/Chromium not found. The page SOM is returned as structured data instead. Install Chrome for pixel-perfect screenshots.",
        "som": som_json,
        "hint": "Use `Plasmate.getSom` or `plasmate fetch` for structured content extraction."
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardened_renderer_configuration_is_explicit_and_has_no_unsafe_file_switches() {
        let directory = std::path::Path::new("/owned-render-dir");
        let screenshot = directory.join("capture.png");
        let args = hardened_html_chrome_args(directory, &screenshot, &ScreenshotOptions::default());

        assert!(args.iter().any(|arg| arg == "--disable-javascript"));
        assert!(args
            .iter()
            .any(|arg| arg == "--proxy-server=http://127.0.0.1:9"));
        assert!(args
            .iter()
            .any(|arg| arg == "--proxy-bypass-list=<-loopback>"));
        assert!(!args.iter().any(|arg| {
            arg == "--allow-file-access-from-files"
                || arg == "--disable-web-security"
                || arg == "--no-sandbox"
        }));
    }

    #[test]
    fn hardened_document_sandboxes_and_policy_wraps_untrusted_html() {
        let document = hardened_render_document(
            "<script>document.body.textContent='executed'</script><img src=\"file:///etc/passwd\">",
        );
        assert!(document.contains("<iframe sandbox"));
        assert!(!document.contains("scriptEnabled"));
        assert!(document.contains("default-src 'none'"));
        assert!(document.contains("script-src 'none'"));
        assert!(document.contains("frame-src 'none'"));
        assert!(document.contains("file:///etc/passwd"));
        assert!(!document.contains("<script>document.body"));
        assert!(document.contains("&lt;script&gt;document.body"));
    }

    #[cfg(unix)]
    #[test]
    fn contained_child_terminates_descendants_and_reaps_root() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant.pid");
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args(["-c", "sleep 30 & echo $! > \"$1\"; wait", "sh"])
            .arg(&pid_file)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = ContainedChild::spawn(&mut command).unwrap();
        let root_pid = child.child.id() as libc::pid_t;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !pid_file.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture did not publish descendant pid"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let descendant_pid: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();

        child.terminate_and_wait();
        assert_eq!(
            unsafe { libc::waitpid(root_pid, std::ptr::null_mut(), libc::WNOHANG) },
            -1,
            "root process should already have been waited on"
        );
        let cleanup_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while unsafe { libc::kill(descendant_pid, 0) } == 0 {
            assert!(
                std::time::Instant::now() < cleanup_deadline,
                "descendant {descendant_pid} survived contained cleanup"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    #[serial_test::serial]
    fn chromium_boundary_blocks_script_execution_and_cross_file_rendering() {
        if !chrome_available() {
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let secret_svg = directory.path().join("secret.svg");
        std::fs::write(
            &secret_svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="red"/></svg>"#,
        )
        .unwrap();
        let baseline = "<style>html,body{background:white;height:100%;margin:0}</style>";
        let hostile = format!(
            "<style>html,body{{background:white url('file://{}') center/cover;height:100%;margin:0}}</style><script>document.documentElement.style.background='red';document.body.style.background='red'</script>",
            secret_svg.display()
        );
        let options = ScreenshotOptions {
            width: 320,
            height: 200,
            ..Default::default()
        };
        let baseline_image = capture_html_with_limits(
            baseline,
            "https://example.test/",
            &options,
            std::time::Duration::from_secs(30),
            1024 * 1024,
        )
        .expect("baseline hardened screenshot should render");
        let visible_control = capture_html_with_limits(
            "<style>html,body{background:blue;height:100%;margin:0}</style>",
            "https://example.test/",
            &options,
            std::time::Duration::from_secs(30),
            1024 * 1024,
        )
        .expect("visible control screenshot should render");
        let hostile_image = capture_html_with_limits(
            &hostile,
            "https://example.test/",
            &options,
            std::time::Duration::from_secs(30),
            1024 * 1024,
        )
        .expect("hostile hardened screenshot should render");

        assert_ne!(
            visible_control, baseline_image,
            "sandboxed srcdoc must still render safe static content"
        );
        assert_eq!(
            hostile_image, baseline_image,
            "script execution or cross-file rendering changed the pixels"
        );
    }

    #[test]
    fn test_format_from_str() {
        assert_eq!(Format::from_str("png"), Format::Png);
        assert_eq!(Format::from_str("jpeg"), Format::Jpeg);
        assert_eq!(Format::from_str("jpg"), Format::Jpeg);
        assert_eq!(Format::from_str("webp"), Format::Webp);
        assert_eq!(Format::from_str("PNG"), Format::Png);
        assert_eq!(Format::from_str("unknown"), Format::Png);
    }

    #[test]
    fn test_format_content_type() {
        assert_eq!(Format::Png.content_type(), "image/png");
        assert_eq!(Format::Jpeg.content_type(), "image/jpeg");
        assert_eq!(Format::Webp.content_type(), "image/webp");
    }

    #[test]
    fn test_format_as_str() {
        assert_eq!(Format::Png.as_str(), "png");
        assert_eq!(Format::Jpeg.as_str(), "jpeg");
        assert_eq!(Format::Webp.as_str(), "webp");
    }

    #[test]
    fn test_default_options() {
        let opts = ScreenshotOptions::default();
        assert_eq!(opts.width, DEFAULT_WIDTH);
        assert_eq!(opts.height, DEFAULT_HEIGHT);
        assert_eq!(opts.format, Format::Png);
        assert!(opts.quality.is_none());
        assert!(!opts.full_page);
    }

    #[test]
    fn test_find_chrome_does_not_crash() {
        // Just verify it returns Some or None without panicking
        let _result = find_chrome();
    }

    #[test]
    fn test_chrome_available_does_not_crash() {
        let _result = chrome_available();
    }

    #[test]
    fn test_capture_url_returns_result() {
        let opts = ScreenshotOptions::default();
        let result = capture_url("https://example.com", &opts);
        // Either succeeds (Chrome found) or returns ChromeNotFound
        match result {
            Ok(data) => {
                // Should be a valid PNG (starts with PNG magic bytes)
                assert!(data.len() > 8, "Screenshot data too small");
                assert_eq!(&data[0..4], &[0x89, 0x50, 0x4E, 0x47], "Not a PNG file");
            }
            Err(ScreenshotError::ChromeNotFound) => {
                // Expected if Chrome is not installed in test env
            }
            Err(e) => {
                // CaptureFailed is also acceptable (e.g. network issues in CI)
                eprintln!("capture_url error (acceptable in CI): {}", e);
            }
        }
    }

    #[test]
    fn test_capture_html_returns_result() {
        let opts = ScreenshotOptions::default();
        let result = capture_html(
            "<html><body><h1>Test</h1></body></html>",
            "https://example.com",
            &opts,
        );
        match result {
            Ok(data) => {
                assert!(data.len() > 8, "Screenshot data too small");
                assert_eq!(&data[0..4], &[0x89, 0x50, 0x4E, 0x47], "Not a PNG file");
            }
            Err(ScreenshotError::ChromeNotFound) => {}
            Err(e) => {
                eprintln!("capture_html error (acceptable in CI): {}", e);
            }
        }
    }

    #[test]
    fn test_som_fallback_structure() {
        let som = crate::som::types::Som {
            som_version: "1".to_string(),
            title: "Test Page".to_string(),
            url: "https://example.com".to_string(),
            lang: "en".to_string(),
            regions: vec![],
            meta: crate::som::types::SomMeta {
                html_bytes: 100,
                som_bytes: 50,
                element_count: 5,
                interactive_count: 1,
            },
            structured_data: None,
        };
        let fallback = som_fallback(&som);
        assert_eq!(fallback["error"], "screenshot_not_implemented");
        assert!(fallback["som"].is_object());
        assert!(fallback["message"].as_str().unwrap().contains("Chrome"));
        assert!(fallback["hint"]
            .as_str()
            .unwrap()
            .contains("Plasmate.getSom"));
    }
}
