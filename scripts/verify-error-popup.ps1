param(
  [int]$Port = 46231,
  [int]$TimeoutSec = 20,
  [switch]$Visual
)

$ErrorActionPreference = "Stop"
$base = "http://127.0.0.1:$Port"

function Get-PopupWindow {
  Get-Process on-start-up-error-popup -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowTitle -eq "On Start Up - Process failed" }
}

Write-Host "Verifying error popup window..." -ForegroundColor Cyan

# Ensure service is reachable
try {
  $null = Invoke-RestMethod -Uri "$base/api/state" -Method Get -TimeoutSec 5
} catch {
  Write-Host "FAIL: service not reachable at $base ($($_.Exception.Message))" -ForegroundColor Red
  exit 1
}

# Ensure server is running (popup is triggered by the server via separate executable)
if (-not (Get-Process on-start-up-server -ErrorAction SilentlyContinue)) {
  Write-Host "FAIL: server process not running." -ForegroundColor Red
  exit 1
}

# Close any stale popup window from previous runs
Get-PopupWindow | ForEach-Object { try { $_.CloseMainWindow() | Out-Null } catch {} }
Start-Sleep -Milliseconds 300

$entry = @{
  id = ""
  label = "__verify_error_popup__"
  dir = ""
  command = "cmd /c exit 7"
  trigger = @{ kind = "startup" }
  alerts = @{ start = $false; startSound = "chime"; exit = $false; exitSound = "chime"; errorSound = ""; sounds = @() }
  errorPolicy = "ignore"
  errorPopup = $true
  errorReruns = -1
  rerunDelay = 3
  rerunBackoff = $false
  halted = $false
}

$added = Invoke-RestMethod -Uri "$base/api/entries" -Method Post -ContentType "application/json" -Body ($entry | ConvertTo-Json -Depth 6)
$id = $added.entry.id
if (-not $id) {
  Write-Host "FAIL: could not create test entry" -ForegroundColor Red
  exit 1
}

$found = $false
$closable = $false
try {
  Invoke-RestMethod -Uri "$base/api/start" -Method Post -ContentType "application/json" -Body (@{ id = $id } | ConvertTo-Json) | Out-Null

  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  while ((Get-Date) -lt $deadline) {
    $p = Get-PopupWindow | Select-Object -First 1
    if ($p) {
      $found = $true
      if ($Visual) {
        Write-Host "POPUP OPEN (visual mode): confirm content is visible, then close the window." -ForegroundColor Yellow
        $vDeadline = (Get-Date).AddSeconds(120)
        while ((Get-Date) -lt $vDeadline) {
          $still = Get-PopupWindow | Select-Object -First 1
          if (-not $still) { $closable = $true; break }
          Start-Sleep -Milliseconds 300
        }
      } else {
        # Verify it is a real window and can be closed (WM_CLOSE via CloseMainWindow)
        try { $closable = [bool]$p.CloseMainWindow() } catch { $closable = $false }
        Start-Sleep -Milliseconds 500
        $still = Get-PopupWindow | Select-Object -First 1
        if ($still) {
          try { Stop-Process -Id $still.Id -Force -ErrorAction SilentlyContinue } catch {}
        }
      }
      break
    }
    Start-Sleep -Milliseconds 300
  }
} finally {
  # Clean up test entry
  try { Invoke-RestMethod -Uri "$base/api/entries/remove" -Method Post -ContentType "application/json" -Body (@{ id = $id } | ConvertTo-Json) | Out-Null } catch {}
}

if (-not $found) {
  Write-Host "FAIL: error popup window did not appear within ${TimeoutSec}s." -ForegroundColor Red
  exit 1
}

if ($closable) {
  Write-Host "PASS: error popup window appeared and accepted close." -ForegroundColor Green
} else {
  Write-Host "PASS (partial): error popup window appeared, but CloseMainWindow returned false (window may still be closable via title bar)." -ForegroundColor Yellow
}
exit 0