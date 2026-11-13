//! Register / unregister the background server so it starts automatically
//! when the user logs in — without any window or console appearing.
//!
//! * Windows: `HKCU\...\CurrentVersion\Run` registry value
//! * macOS: `~/Library/LaunchAgents` launch agent
//! * Linux: `~/.config/autostart` desktop entry

use std::path::Path;

/// Make `exe` launch at login. Re-registering is cheap and keeps the entry
/// correct if the app was moved.
pub fn install(exe: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let quoted = format!("\"{}\"", exe.display());
        run_reg(&[
            "add",
            RUN_KEY,
            "/v",
            VALUE_NAME,
            "/t",
            "REG_SZ",
            "/d",
            &quoted,
            "/f",
        ])
        .map(|_| ())
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        let dir = Path::new(&home).join("Library/LaunchAgents");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{IDENTIFIER}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
"#,
            IDENTIFIER = crate::IDENTIFIER,
            exe = exe.display(),
        );
        std::fs::write(dir.join(format!("{}.plist", crate::IDENTIFIER)), plist).map_err(|e| e.to_string())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        let dir = Path::new(&home).join(".config/autostart");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName=On Start Up\nComment=Process auto-launcher\nExec={exe}\nX-GNOME-Autostart-enabled=true\n",
            exe = exe.display(),
        );
        std::fs::write(dir.join("on-start-up.desktop"), desktop).map_err(|e| e.to_string())
    }
}

/// Remove the login item. Missing entries are treated as success.
pub fn uninstall() -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = run_reg(&["delete", RUN_KEY, "/v", VALUE_NAME, "/f"]);
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        let path = Path::new(&home).join(format!("Library/LaunchAgents/{}.plist", crate::IDENTIFIER));
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(Path::new(&home).join(".config/autostart/on-start-up.desktop"));
        Ok(())
    }
}

/// Whether the app is currently registered to start at login.
pub fn status() -> Result<bool, String> {
    #[cfg(windows)]
    {
        Ok(run_reg(&["query", RUN_KEY, "/v", VALUE_NAME]).is_ok())
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        Ok(Path::new(&home)
            .join(format!("Library/LaunchAgents/{}.plist", crate::IDENTIFIER))
            .is_file())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        Ok(Path::new(&home)
            .join(".config/autostart/on-start-up.desktop")
            .is_file())
    }
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE_NAME: &str = "OnStartUp";

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
