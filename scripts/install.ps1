# on-start-up installer for Windows
# Usage: iwr -useb https://on-start-up.seremtitus.co.ke/install.ps1 | iex

$ErrorActionPreference = "Stop"

$App = "on-start-up"
$AppName = "On Start Up"
$GitHubRepo = "SeremTitus/on-start-up"
$InstallDir = if ($env:ON_START_UP_INSTALL_DIR) { $env:ON_START_UP_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "$App\bin" }

function Info($msg)  { Write-Host "OK: $msg" -ForegroundColor Green }
function Warn($msg)  { Write-Host "!  $msg" -ForegroundColor Yellow }
function Fail($msg)  { Write-Host "X  $msg" -ForegroundColor Red; exit 1 }

# Detect architecture
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    "AMD64" { "x86_64" }
    "ARM64" { "aarch64" }
    default { Fail "Unsupported architecture: $($env:PROCESSOR_ARCHITECTURE)" }
}

if ($arch -ne "x86_64") {
    Fail "Unsupported architecture: $arch (release builds: windows-x86_64)"
}

$archive = "$App-windows-x86_64.zip"
$url = "https://github.com/$GitHubRepo/releases/latest/download/$archive"

Info "Downloading $archive ..."
$tmp = Join-Path ([IO.Path]::GetTempPath()) "$App-install-$(Get-Random)"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    Invoke-WebRequest -Uri $url -OutFile (Join-Path $tmp $archive) -UseBasicParsing
} catch {
    Fail "Download failed: $url ($_)"
}

Info "Extracting ..."
$extract = Join-Path $tmp "out"
Expand-Archive -Path (Join-Path $tmp $archive) -DestinationPath $extract -Force

Info "Installing to $InstallDir ..."
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
foreach ($bin in @("$App.exe", "$App-server.exe", "$App-error-popup.exe")) {
    $src = Get-ChildItem -Path $extract -Recurse -Filter $bin | Select-Object -First 1
    if (-not $src) { Fail "Missing $bin in the archive" }
    Copy-Item $src.FullName -Destination $InstallDir -Force
    Info "  installed $bin"
}

# Write uninstall script
$uninstallPath = Join-Path $InstallDir "uninstall.ps1"
$shortcutPath = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\$AppName.lnk"
$uninstallScript = @"
# on-start-up uninstaller for Windows
`$ErrorActionPreference = "SilentlyContinue"

`$App = "$App"
`$InstallDir = "$InstallDir"
`$Shortcut = "$shortcutPath"

# Stop running app/service
try { Invoke-WebRequest -Uri "http://127.0.0.1:46231/api/shutdown" -UseBasicParsing -Method Post | Out-Null } catch {}
Start-Sleep -Milliseconds 800
Get-Process "$App","$App-server","$App-error-popup" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

# Remove Start Menu shortcut
if (Test-Path `$Shortcut) { Remove-Item `$Shortcut -Force }

# Remove installed binaries and this script
Remove-Item (Join-Path `$InstallDir "$App.exe") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path `$InstallDir "$App-server.exe") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path `$InstallDir "$App-error-popup.exe") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path `$InstallDir "uninstall.ps1") -Force -ErrorAction SilentlyContinue

# Remove install dir if empty
if (Test-Path `$InstallDir) {
    `$left = Get-ChildItem `$InstallDir -Force -ErrorAction SilentlyContinue
    if (-not `$left) { Remove-Item `$InstallDir -Force -ErrorAction SilentlyContinue }
}

Write-Host "OK: On Start Up uninstalled." -ForegroundColor Green
"@
Set-Content -Path $uninstallPath -Value $uninstallScript -Encoding UTF8
Info "  installed uninstall.ps1"

# Add Start Menu shortcut
$startMenuDir = Split-Path $shortcutPath -Parent
New-Item -ItemType Directory -Force -Path $startMenuDir | Out-Null
$targetExe = Join-Path $InstallDir "$App.exe"
$WshShell = New-Object -ComObject WScript.Shell
$shortcut = $WshShell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $targetExe
$shortcut.WorkingDirectory = $InstallDir
$shortcut.Description = $AppName
$shortcut.IconLocation = "$targetExe,0"
$shortcut.Save()
Info "Added Start Menu shortcut: $shortcutPath"

# Cleanup temp
Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue

Info "Done. Launch '$AppName' from the Start Menu."
Write-Host ""
Write-Host "  It opens the UI, starts the background service and registers itself"
Write-Host "  to launch at login (toggle in the UI: 'start with system')."
Write-Host "  To uninstall: powershell -ExecutionPolicy Bypass -File `"$uninstallPath`""
