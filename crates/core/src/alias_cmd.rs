//! Register/unregister the `osu` command alias shim on the user's PATH.

use std::path::{Path, PathBuf};

/// Directory containing the command alias shim.
pub fn bin_dir() -> PathBuf {
    crate::config_dir().join("bin")
}

pub fn normalize_command_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() { "osu".to_string() } else { trimmed.to_string() }
}

pub fn validate_command_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("alias command name is required".into());
    }
    if name.len() > 32 {
        return Err("alias command name is too long".into());
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if !ok {
        return Err("alias command name may contain only letters, numbers, '.', '-' and '_'".into());
    }
    Ok(())
}

#[cfg(windows)]
fn shim_cmd_path(name: &str) -> PathBuf {
    bin_dir().join(format!("{}.cmd", name))
}

#[cfg(windows)]
fn shim_ps1_path(name: &str) -> PathBuf {
    bin_dir().join(format!("{}.ps1", name))
}

/// Install the command shim and add its bin directory to the user PATH.
pub fn install(name: &str) -> Result<(), String> {
    let name = normalize_command_name(name);
    validate_command_name(&name)?;
    let dir = bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        write_shims(&dir, &name)?;
        add_to_user_path(&dir)?;
    }
    #[cfg(unix)]
    {
        write_shims_unix(&dir, &name)?;
        add_to_user_path_unix(&dir)?;
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = name;
        return Err("alias activation is not supported on this platform".into());
    }
    Ok(())
}

/// Remove the command shim from PATH; if this was the last shim, remove bin dir from PATH.
pub fn uninstall(name: &str) -> Result<(), String> {
    let name = normalize_command_name(name);
    let dir = bin_dir();
    #[cfg(windows)]
    {
        let _ = std::fs::remove_file(shim_cmd_path(&name));
        let _ = std::fs::remove_file(shim_ps1_path(&name));
        let has_other = std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flat_map(|r| r.filter_map(|e| e.ok()))
            .any(|e| {
                let p = e.path();
                matches!(p.extension().and_then(|s| s.to_str()), Some("cmd") | Some("ps1"))
            });
        if !has_other {
            remove_from_user_path(&dir)?;
        }
    }
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(dir.join(&name));
        let has_other = std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flat_map(|r| r.filter_map(|e| e.ok()))
            .any(|e| e.path().is_file());
        if !has_other {
            remove_from_user_path_unix(&dir)?;
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = name;
    }
    Ok(())
}

/// Whether the shim bin dir is currently present on the user PATH.
pub fn is_active(_name: &str) -> Result<bool, String> {
    let dir = bin_dir();
    #[cfg(windows)]
    {
        let path = read_user_path()?;
        return Ok(path_entries(&path).iter().any(|p| same_path(p, &dir)));
    }
    #[cfg(unix)]
    {
        return Ok(is_on_user_path_unix(&dir));
    }
    #[cfg(not(any(windows, unix)))]
    {
        Ok(false)
    }
}

#[cfg(windows)]
fn write_shims(_dir: &Path, name: &str) -> Result<(), String> {
    let config = crate::config_dir().join("on-start-up.json");
    let ps1 = format!(
        r#"$ErrorActionPreference = 'Stop'
$configPath = '{config}'
if ($args.Count -lt 1) {{
  Write-Error 'Usage: {cmd_name} <alias> [args...]'
  exit 1
}}
$name = [string]$args[0]
$rest = @()
if ($args.Count -gt 1) {{ $rest = $args[1..($args.Count - 1)] }}
if (-not (Test-Path -LiteralPath $configPath)) {{
  Write-Error "Config not found: $configPath"
  exit 1
}}
$cfg = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
$alias = $null
foreach ($a in @($cfg.aliases)) {{
  if ($a.name -ieq $name) {{ $alias = $a; break }}
}}
if ($null -eq $alias) {{
  Write-Error "Unknown alias: $name"
  exit 1
}}
$commands = @($alias.commands)
if ($commands.Count -lt 1) {{
  Write-Error "Alias has no commands: $name"
  exit 1
}}
$workDir = [string]$alias.dir
if ([string]::IsNullOrWhiteSpace($workDir)) {{ $workDir = (Get-Location).Path }}
if (-not (Test-Path -LiteralPath $workDir)) {{
  Write-Error "Alias directory not found: $workDir"
  exit 1
}}
$quotedRest = @()
if ($rest.Count -gt 0) {{
  $quotedRest = $rest | ForEach-Object {{
    $s = [string]$_
    if ($s -match '[\s"]') {{ '"' + ($s -replace '"', '\"') + '"' }} else {{ $s }}
  }}
}}
Push-Location -LiteralPath $workDir
try {{
  foreach ($step in $commands) {{
    $cmd = [string]$step.command
    if ([string]::IsNullOrWhiteSpace($cmd)) {{ continue }}
    $accepts = $false
    if ($null -ne $step.acceptsArgs) {{ $accepts = [bool]$step.acceptsArgs }}
    if ($accepts -and $quotedRest.Count -gt 0) {{
      $cmd = $cmd + ' ' + ($quotedRest -join ' ')
    }}
    & cmd.exe /d /s /c $cmd
    if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}
  }}
  exit 0
}} finally {{
  Pop-Location
}}
"#,
        config = ps1_escape(&config.to_string_lossy()),
        cmd_name = ps1_escape(name),
    );
    std::fs::write(shim_ps1_path(name), ps1).map_err(|e| e.to_string())?;

    let cmd = format!(
        "@echo off\r\npowershell -NoProfile -ExecutionPolicy Bypass -File \"%~dp0{}.ps1\" %*\r\n",
        name
    );
    std::fs::write(shim_cmd_path(name), cmd).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(windows)]
fn ps1_escape(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(windows)]
fn read_user_path() -> Result<String, String> {
    let out = run_reg(&["query", ENV_KEY, "/v", "Path"])?;
    // reg query output: "    Path    REG_EXPAND_SZ    ..."
    for line in out.lines() {
        let line = line.trim();
        if line.to_lowercase().starts_with("path") {
            // split by whitespace, find REG_* token, rest is value
            if let Some(idx) = line.find("REG_") {
                let rest = &line[idx..];
                if let Some(space) = rest.find(|c: char| c.is_whitespace()) {
                    return Ok(rest[space..].trim().to_string());
                }
            }
        }
    }
    Ok(String::new())
}

#[cfg(windows)]
fn write_user_path(value: &str) -> Result<(), String> {
    run_reg(&[
        "add",
        ENV_KEY,
        "/v",
        "Path",
        "/t",
        "REG_EXPAND_SZ",
        "/d",
        value,
        "/f",
    ])
    .map(|_| ())
}

#[cfg(windows)]
fn add_to_user_path(dir: &Path) -> Result<(), String> {
    let current = read_user_path().unwrap_or_default();
    let mut entries = path_entries(&current);
    if entries.iter().any(|p| same_path(p, dir)) {
        return Ok(());
    }
    entries.push(dir.to_string_lossy().to_string());
    let new_path = entries.join(";");
    write_user_path(&new_path)
}

#[cfg(windows)]
fn remove_from_user_path(dir: &Path) -> Result<(), String> {
    let current = read_user_path().unwrap_or_default();
    let entries: Vec<String> = path_entries(&current)
        .into_iter()
        .filter(|p| !same_path(p, dir))
        .collect();
    let new_path = entries.join(";");
    write_user_path(&new_path)
}

#[cfg(windows)]
fn path_entries(path: &str) -> Vec<String> {
    path.split(';')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(windows)]
fn same_path(a: &str, b: &Path) -> bool {
    let a = a.trim().trim_matches('"').trim_end_matches(['\\', '/']);
    let b = b.to_string_lossy();
    let b = b.trim().trim_end_matches(['\\', '/']);
    a.eq_ignore_ascii_case(b)
}

#[cfg(windows)]
const ENV_KEY: &str = r"HKCU\Environment";

#[cfg(windows)]
fn run_reg(args: &[&str]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let output = std::process::Command::new("reg")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(unix)]
fn write_shims_unix(_dir: &Path, name: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let config = crate::config_dir().join("on-start-up.json");
    let script = format!(
        r#"#!/usr/bin/env python3
import json, os, shlex, subprocess, sys
config_path = r"{config}"
if len(sys.argv) < 2:
    print("Usage: {cmd_name} <alias> [args...]", file=sys.stderr)
    sys.exit(1)
name = sys.argv[1]
rest = sys.argv[2:]
try:
    with open(config_path, "r", encoding="utf-8") as f:
        cfg = json.load(f)
except Exception as e:
    print(f"Config not found or invalid: {{config_path}} ({{e}})", file=sys.stderr)
    sys.exit(1)
alias = None
for a in (cfg.get("aliases") or []):
    if str(a.get("name", "")).lower() == name.lower():
        alias = a
        break
if alias is None:
    print(f"Unknown alias: {{name}}", file=sys.stderr)
    sys.exit(1)
commands = alias.get("commands") or []
if not commands:
    print(f"Alias has no commands: {{name}}", file=sys.stderr)
    sys.exit(1)
workdir = str(alias.get("dir") or "").strip() or os.getcwd()
if not os.path.isdir(workdir):
    print(f"Alias directory not found: {{workdir}}", file=sys.stderr)
    sys.exit(1)
for step in commands:
    cmd = str((step or {{}}).get("command") or "")
    if not cmd.strip():
        continue
    accepts = bool((step or {{}}).get("acceptsArgs"))
    if accepts and rest:
        cmd = cmd + " " + " ".join(shlex.quote(x) for x in rest)
    p = subprocess.run(cmd, shell=True, cwd=workdir)
    if p.returncode != 0:
        sys.exit(p.returncode)
sys.exit(0)
"#,
        config = config.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""),
        cmd_name = name.replace('\\', "\\\\").replace('"', "\\\""),
    );
    let path = bin_dir().join(name);
    std::fs::write(&path, script).map_err(|e| e.to_string())?;
    let mut perms = std::fs::metadata(&path)
        .map_err(|e| e.to_string())?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(unix)]
fn unix_profile_files() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut files = vec![
        home.join(".profile"),
        home.join(".zprofile"),
        home.join(".bash_profile"),
    ];
    // keep only unique existing parents (all have home parent) and unique paths
    files.dedup();
    files
}

#[cfg(unix)]
fn path_export_line(dir: &Path) -> String {
    format!(
        "export PATH=\"{}:$PATH\" # on-start-up alias bin",
        dir.to_string_lossy()
    )
}

#[cfg(unix)]
fn add_to_user_path_unix(dir: &Path) -> Result<(), String> {
    let line = path_export_line(dir);
    for file in unix_profile_files() {
        let mut contents = std::fs::read_to_string(&file).unwrap_or_default();
        if contents.lines().any(|l| l.contains("# on-start-up alias bin") && l.contains(&*dir.to_string_lossy())) {
            continue;
        }
        // Remove stale on-start-up alias bin lines for other dirs? keep simple: only skip identical.
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(&line);
        contents.push('\n');
        std::fs::write(&file, contents).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(unix)]
fn remove_from_user_path_unix(dir: &Path) -> Result<(), String> {
    let dir_str = dir.to_string_lossy().to_string();
    for file in unix_profile_files() {
        let Ok(contents) = std::fs::read_to_string(&file) else {
            continue;
        };
        let mut changed = false;
        let new_contents: String = contents
            .lines()
            .filter(|l| {
                let remove = l.contains("# on-start-up alias bin") && l.contains(&dir_str);
                if remove { changed = true; }
                !remove
            })
            .map(|l| format!("{l}\n"))
            .collect();
        if changed {
            std::fs::write(&file, new_contents).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_on_user_path_unix(dir: &Path) -> bool {
    let dir_str = dir.to_string_lossy().to_string();
    for file in unix_profile_files() {
        let Ok(contents) = std::fs::read_to_string(&file) else {
            continue;
        };
        if contents.lines().any(|l| l.contains("# on-start-up alias bin") && l.contains(&dir_str)) {
            return true;
        }
    }
    // Also check current PATH env
    if let Ok(path) = std::env::var("PATH") {
        return path.split(':').any(|p| p == dir_str);
    }
    false
}
