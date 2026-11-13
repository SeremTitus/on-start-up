//! Development automation, run as `cargo xtask …` (see `.cargo/config.toml`).
//!
//! `cargo xtask build [--release|--debug]`
//!     Builds the frontend (bun) and both Rust binaries. Debug by default.
//!
//! `cargo xtask start [--release|--debug]`
//!     Kills existing app processes, builds, then starts backend and UI.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("build") => {
            let release = match parse_profile_flags(&args[1..], false) {
                Ok(r) => r,
                Err(code) => return code,
            };
            build(release)
        }
        Some("start") => {
            let release = match parse_profile_flags(&args[1..], true) {
                Ok(r) => r,
                Err(code) => return code,
            };
            start(release)
        }
        Some("--help") | Some("-h") | Some("help") => {
            usage();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown task `{other}`\n");
            usage();
            ExitCode::FAILURE
        }
        None => {
            usage();
            ExitCode::SUCCESS
        }
    }
}

fn parse_profile_flags(args: &[String], default_release: bool) -> Result<bool, ExitCode> {
    let mut release = default_release;
    for arg in args {
        match arg.as_str() {
            "--release" => release = true,
            "--debug" => release = false,
            other => {
                eprintln!("error: unknown flag `{other}`\n");
                usage();
                return Err(ExitCode::FAILURE);
            }
        }
    }
    Ok(release)
}

fn usage() {
    eprintln!(
        "usage: cargo xtask <task>\n\n\
         tasks:\n\
         \x20 build [--release|--debug]   build frontend + server + UI (debug by default)\n\
         \x20 start [--release|--debug]   kill existing, build, then start backend and UI (release by default)"
    );
}

/// Workspace root (`…/on-start-up`), given that this crate lives in `xtask/`.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask crate is not at the workspace root")
        .to_path_buf()
}

fn run(root: &Path, program: &str, args: &[&str]) -> Result<(), ExitCode> {
    println!("\n==> {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|e| {
            eprintln!("error: failed to run `{program}`: {e}");
            ExitCode::FAILURE
        })?;
    if status.success() {
        Ok(())
    } else {
        eprintln!("error: `{program} {}` failed", args.join(" "));
        Err(ExitCode::FAILURE)
    }
}

fn build(release: bool) -> ExitCode {
    let root = workspace_root();
    let profile = if release { "release" } else { "debug" };
    println!("building on-start-up ({profile})");

    let mut server_args = vec!["build", "-p", "on-start-up-server"];
    let mut popup_args = vec!["build", "-p", "on-start-up-error-popup"];
    let mut ui_args = vec![
        "build",
        "-p",
        "on-start-up",
        // Embeds the frontend so the UI binary is standalone. Without it the
        // binary is a dev build that only works against `bun run dev`.
        "--features",
        "custom-protocol",
    ];
    if release {
        server_args.push("--release");
        popup_args.push("--release");
        ui_args.push("--release");
    }

    let steps: Vec<(&str, Vec<&str>)> = vec![
        ("bun", vec!["install"]),
        ("bun", vec!["run", "build"]),
        ("cargo", server_args),
        ("cargo", popup_args),
        ("cargo", ui_args),
    ];

    for (program, args) in &steps {
        if let Err(code) = run(&root, program, args) {
            return code;
        }
    }

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let root_display = root.display();
    println!(
        "\ndone ({profile}):\n  {root_display}/target/{profile}/on-start-up-server{ext}\n  {root_display}/target/{profile}/on-start-up-error-popup{ext}\n  {root_display}/target/{profile}/on-start-up{ext}"
    );
    println!("\nbinaries are standalone (the UI embeds its frontend).");
    ExitCode::SUCCESS
}

fn kill_existing() {
    println!("\n==> stopping existing on-start-up processes");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        for im in [
            "on-start-up.exe",
            "on-start-up-server.exe",
            "on-start-up-error-popup.exe",
        ] {
            let _ = Command::new("taskkill")
                .args(["/IM", im, "/F", "/T"])
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("pkill")
            .args(["-f", "on-start-up-server"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("pkill")
            .args(["-f", "on-start-up-error-popup"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("pkill")
            .args(["-x", "on-start-up"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn spawn_detached(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        Command::new(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Command::new(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(())
    }
}

fn start(release: bool) -> ExitCode {
    let root = workspace_root();
    let profile = if release { "release" } else { "debug" };

    kill_existing();
    std::thread::sleep(std::time::Duration::from_millis(300));

    let build_code = build(release);
    if build_code != ExitCode::SUCCESS {
        return build_code;
    }

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let bin_dir = root.join("target").join(profile);
    let server = bin_dir.join(format!("on-start-up-server{ext}"));
    let ui = bin_dir.join(format!("on-start-up{ext}"));

    for (label, path) in [("server", &server), ("ui", &ui)] {
        if !path.is_file() {
            eprintln!("error: {label} binary not found at {}", path.display());
            return ExitCode::FAILURE;
        }
    }

    println!("\n==> starting backend: {}", server.display());
    if let Err(e) = spawn_detached(&server) {
        eprintln!("error: failed to start server: {e}");
        return ExitCode::FAILURE;
    }

    // Give the server a moment to bind before the UI connects.
    std::thread::sleep(std::time::Duration::from_millis(800));

    println!("==> starting ui: {}", ui.display());
    if let Err(e) = spawn_detached(&ui) {
        eprintln!("error: failed to start ui: {e}");
        return ExitCode::FAILURE;
    }

    println!("\nstarted backend and ui ({profile}).");
    ExitCode::SUCCESS
}
