# ==============================================================================
# Fastty installation script for Windows (PowerShell)
# ==============================================================================
# Run directly from PowerShell with:
# irm https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.ps1 | iex
# ==============================================================================

$ErrorActionPreference = 'Stop'
$GitHubUser = "diegoleteliers10"
$GitHubRepo = "fasty"
$AppName    = "fastty"
$BinaryName = "fastty.exe"

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

Write-Host "=== Starting $AppName installation on Windows ===" -ForegroundColor Green

# ── 1. Resolve platform architecture and directories ──────────────────────────
$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -eq "ARM64") {
    $Target = "aarch64-pc-windows-msvc"
} else {
    $Target = "x86_64-pc-windows-msvc"
}

$ConfigDir = Join-Path $env:APPDATA     "fastty\config"
$DataDir   = Join-Path $env:APPDATA     "fastty\data"
$StateDir  = Join-Path $env:LOCALAPPDATA "fastty\state"
$CacheDir  = Join-Path $env:LOCALAPPDATA "fastty\cache"
$BinDir    = Join-Path $env:LOCALAPPDATA "fastty\bin"

# ── 2. Create directories ──────────────────────────────────────────────────────
foreach ($dir in @($ConfigDir, $DataDir, $StateDir, $CacheDir, $BinDir)) {
    if (-not (Test-Path $dir)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }
}

# ── 3. Query GitHub API for latest release ────────────────────────────────────
Write-Host "Fetching latest release version from GitHub API..." -ForegroundColor Cyan
$ApiUrl = "https://api.github.com/repos/$GitHubUser/$GitHubRepo/releases/latest"

try {
    $oldProgressPreference = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'

    $Response = Invoke-RestMethod -Uri $ApiUrl -Method Get
    $LatestTag = $Response.tag_name
} catch {
    Write-Error "ERROR: Could not fetch the latest version from GitHub ($ApiUrl)."
    $ProgressPreference = $oldProgressPreference
    exit 1
}

Write-Host "Latest version found: $LatestTag (Target: $Target)" -ForegroundColor Green

# ── 4. Download Windows archive asset ─────────────────────────────────────────
$ZipName = "$AppName-$Target.zip"
$DownloadUrl = "https://github.com/$GitHubUser/$GitHubRepo/releases/download/$LatestTag/$ZipName"

$TempDir = Join-Path $env:TEMP "install-$AppName"
if (Test-Path $TempDir) {
    Remove-Item -Recurse -Force $TempDir
}
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

$ZipPath = Join-Path $TempDir $ZipName
$ExtractDir = Join-Path $TempDir "extracted"

Write-Host "Downloading $ZipName..." -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -UseBasicParsing
} catch {
    Write-Error "ERROR: Failed to download the release asset from $DownloadUrl"
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# ── 5. Extract files ───────────────────────────────────────────────────────────
Write-Host "Extracting files..." -ForegroundColor Cyan
try {
    Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir -Force
} catch {
    Write-Error "ERROR: Failed to extract the ZIP archive."
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# ── 6. Install executable binary ──────────────────────────────────────────────
$ExeSourcePath = Join-Path $ExtractDir $BinaryName
if (-not (Test-Path $ExeSourcePath)) {
    # If nested in a folder
    $Found = Get-ChildItem -Path $ExtractDir -Filter $BinaryName -Recurse | Select-Object -First 1
    if ($Found) {
        $ExeSourcePath = $Found.FullName
    }
}

$ExeDestPath = Join-Path $BinDir $BinaryName

Write-Host "Installing executable to $ExeDestPath..." -ForegroundColor Cyan
try {
    if (Test-Path $ExeDestPath) {
        $OldExePath = "$ExeDestPath.old"
        if (Test-Path $OldExePath) {
            Remove-Item -Path $OldExePath -Force -ErrorAction SilentlyContinue
        }
        Rename-Item -Path $ExeDestPath -NewName "$BinaryName.old" -Force -ErrorAction SilentlyContinue
    }
    Copy-Item -Path $ExeSourcePath -Destination $ExeDestPath -Force
} catch {
    Write-Error "ERROR: Failed to copy executable to ${BinDir}: $_"
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# ── 6.5. Download Application Icon (.ico) ──────────────────────────────────────
$IconUrl = "https://raw.githubusercontent.com/$GitHubUser/$GITHUBRepo/main/assets/fasttyIcon.ico"
$IconDestPath = Join-Path $DataDir "fastty.ico"
try {
    Invoke-WebRequest -Uri $IconUrl -OutFile $IconDestPath -UseBasicParsing -ErrorAction SilentlyContinue
    Copy-Item -Path $IconDestPath -Destination (Join-Path $BinDir "fastty.ico") -Force -ErrorAction SilentlyContinue
} catch {}

# ── 6.6. Initialize default configuration if absent ───────────────────────────
$DestConfigToml = Join-Path $ConfigDir "fastty.toml"
$DestConfigJson = Join-Path $ConfigDir "config.json"
$LegacyConfigToml = Join-Path $ConfigDir "config.toml"

if (-not (Test-Path $DestConfigToml) -and -not (Test-Path $LegacyConfigToml) -and -not (Test-Path $DestConfigJson)) {
    $SrcConfigToml = Join-Path $ExtractDir "fastty.toml"
    if (Test-Path $SrcConfigToml) {
        Copy-Item -Path $SrcConfigToml -Destination $DestConfigToml -Force
    } else {
        $DefaultConfigContent = @'
# fastty configuration file
theme = "default"
opacity = 1.0
scrollback = 1000
session_restore = true
copy_on_select = false
notify_on_command_finish = true

[font]
family = "Cascadia Code"
size = 14.0
weight = 400.0
ligatures = true

[bottombar]
enabled = true
layout = "balanced"
left_widgets = ["git_branch", "git_status"]
right_widgets = ["cwd", "duration", "exit_code"]

[keybindings]
'@
        Set-Content -Path $DestConfigToml -Value $DefaultConfigContent -Encoding UTF8
    }
}

$ProgressPreference = $oldProgressPreference

# ── 7. Register Windows App Path (Win+R / Start execution) ─────────────────────
try {
    $AppPathsKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\fastty.exe"
    if (-not (Test-Path $AppPathsKey)) {
        New-Item -Path $AppPathsKey -Force | Out-Null
    }
    Set-ItemProperty -Path $AppPathsKey -Name "(Default)" -Value $ExeDestPath
    Set-ItemProperty -Path $AppPathsKey -Name "Path" -Value $BinDir
} catch {}

# ── 8. Add to user PATH permanently ────────────────────────────────────────────
$UserPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
$NormalizedBinDir = $BinDir.TrimEnd('\')

$IsInPath = $false
if ($UserPath) {
    $PathList = $UserPath -split ";"
    foreach ($p in $PathList) {
        if ($p.Trim().TrimEnd('\') -eq $NormalizedBinDir) {
            $IsInPath = $true
            break
        }
    }
}

if (-not $IsInPath) {
    Write-Host "Adding $BinDir to User PATH..." -ForegroundColor Yellow
    $NewUserPath = $UserPath
    if ($NewUserPath -and -not $NewUserPath.EndsWith(";")) {
        $NewUserPath += ";"
    }
    $NewUserPath += $NormalizedBinDir

    [Environment]::SetEnvironmentVariable("Path", $NewUserPath, [EnvironmentVariableTarget]::User)
    $env:Path += ";" + $NormalizedBinDir
}

# ── 9. Start Menu shortcut ─────────────────────────────────────────────────────
try {
    $StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    $ShortcutPath = Join-Path $StartMenuDir "Fastty.lnk"

    $WshShell = New-Object -ComObject WScript.Shell
    $Shortcut = $WshShell.CreateShortcut($ShortcutPath)
    $Shortcut.TargetPath = $ExeDestPath
    $Shortcut.WorkingDirectory = $HOME
    if (Test-Path $IconDestPath) {
        $Shortcut.IconLocation = "$IconDestPath,0"
    } else {
        $Shortcut.IconLocation = "$ExeDestPath,0"
    }
    $Shortcut.Description = "Fastty Terminal Emulator"
    $Shortcut.Save()

    Write-Host "Start Menu shortcut registered." -ForegroundColor Green
} catch {}

# ── 10. Final cleanup ──────────────────────────────────────────────────────────
Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue

# ── 11. Summary ────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host "=== Fastty installation complete ===" -ForegroundColor Green
Write-Host "    ✓ Binary  -> $ExeDestPath"
Write-Host "    ✓ Config  -> $ConfigDir"
Write-Host "    ✓ Data    -> $DataDir"
Write-Host "    ✓ State   -> $StateDir"
Write-Host "    ✓ Cache   -> $CacheDir"
Write-Host ""
Write-Host "You can now launch Fastty by typing 'fastty' in terminal or Win+R."
