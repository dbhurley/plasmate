use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

struct Options {
    mode: String,
    bytes: usize,
    exit_code: i32,
    path: Option<PathBuf>,
}

fn parse_options() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let mut mode = None;
    let mut bytes = 0;
    let mut exit_code = 7;
    let mut path = None;

    while let Some(arg) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--mode" => mode = Some(value),
            "--bytes" => bytes = value.parse().map_err(|_| "invalid --bytes".to_string())?,
            "--exit-code" => {
                exit_code = value
                    .parse()
                    .map_err(|_| "invalid --exit-code".to_string())?
            }
            "--path" => path = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }

    Ok(Options {
        mode: mode.ok_or_else(|| "--mode is required".to_string())?,
        bytes,
        exit_code,
        path,
    })
}

fn prepare_supervised_worker() -> Result<(), String> {
    if std::env::var_os("PLASMATE_SUPERVISED_WORKER").is_none() {
        return Ok(());
    }

    #[cfg(unix)]
    unsafe {
        extern "C" {
            fn setpgid(pid: i32, pgid: i32) -> i32;
        }
        if setpgid(0, 0) != 0 {
            return Err(format!(
                "setpgid failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    std::env::remove_var("PLASMATE_SUPERVISED_WORKER");
    std::env::remove_var("PLASMATE_WORKER_MEMORY_LIMIT_BYTES");
    Ok(())
}

fn run(options: Options) -> Result<(), String> {
    match options.mode.as_str() {
        "ok" => println!("ok"),
        "exit" => std::process::exit(options.exit_code),
        "abort" => std::process::abort(),
        "hang" => std::thread::sleep(Duration::from_secs(60)),
        "output" => {
            let chunk = vec![b'x'; options.bytes];
            std::io::stdout()
                .write_all(&chunk)
                .map_err(|error| error.to_string())?;
            std::io::stderr()
                .write_all(&chunk)
                .map_err(|error| error.to_string())?;
        }
        "descendant-hang" => {
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let child = std::process::Command::new(executable)
                .args(["--mode", "hang"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|error| error.to_string())?;
            let path = options
                .path
                .ok_or_else(|| "--path is required for descendant-hang".to_string())?;
            std::fs::write(path, child.id().to_string()).map_err(|error| error.to_string())?;
            std::thread::sleep(Duration::from_secs(60));
        }
        "pid-file-hang" => {
            let path = options
                .path
                .ok_or_else(|| "--path is required for pid-file-hang".to_string())?;
            std::fs::write(path, std::process::id().to_string())
                .map_err(|error| error.to_string())?;
            std::thread::sleep(Duration::from_secs(60));
        }
        mode => return Err(format!("unknown mode: {mode}")),
    }
    Ok(())
}

fn main() {
    let result = prepare_supervised_worker()
        .and_then(|()| parse_options())
        .and_then(run);
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
