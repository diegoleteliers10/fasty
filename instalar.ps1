# ==============================================================================
# Fastty installation script for Windows (PowerShell)
# ==============================================================================
# Run directly from PowerShell with:
# irm https://raw.githubusercontent.com/diegoleteliers10/fastty/main/instalar.ps1 | iex
# ==============================================================================

$GitHubUser = "diegoleteliers10"
$GitHubRepo = "fastty"
$AppName    = "fastty"
$BinaryName = "fastty.exe"

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

Write-Host "=== Starting $AppName installation on Windows ===" -ForegroundColor Green

# ── 1. Resolve platform directories ────────────────────────────────────────────
$ConfigDir = Join-Path $env:APPDATA     "fastty\config"
$DataDir   = Join-Path $env:APPDATA     "fastty\data"
$StateDir  = Join-Path $env:LOCALAPPDATA "fastty\state"
$CacheDir  = Join-Path $env:LOCALAPPDATA "fastty\cache"
$BinDir    = Join-Path $env:LOCALAPPDATA "fastty\bin"

# ── 2. Create directories ──────────────────────────────────────────────────────
foreach ($dir in @($ConfigDir, $DataDir, $StateDir, $CacheDir, $BinDir)) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

# ── 3. Query the GitHub API for the latest release ─────────────────────────────
Write-Host "Fetching the latest version from the GitHub API..." -ForegroundColor Cyan
$ApiUrl = "https://api.github.com/repos/$GitHubUser/$GitHubRepo/releases/latest"

try {
    $oldProgressPreference = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'

    $Response = Invoke-RestMethod -Uri $ApiUrl -Method Get
    $LatestTag = $Response.tag_name
} catch {
    Write-Error "ERROR: Could not fetch the latest version from the GitHub API. Check your connection or the repository."
    $ProgressPreference = $oldProgressPreference
    exit 1
}

Write-Host "Latest version found: $LatestTag" -ForegroundColor Green

# ── 4. Download the Windows .zip asset ─────────────────────────────────────────
$Target = "x86_64-pc-windows-msvc"
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
    Write-Error "ERROR: Failed to download the file from $DownloadUrl"
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# ── 5. Extract ─────────────────────────────────────────────────────────────────
Write-Host "Extracting files..." -ForegroundColor Cyan
try {
    Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir -Force
} catch {
    Write-Error "ERROR: Failed to extract the ZIP archive."
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# ── 6. Install binary ─────────────────────────────────────────────────────────
$ExeSourcePath = Join-Path $ExtractDir $BinaryName
$ExeDestPath = Join-Path $BinDir $BinaryName

Write-Host "Copying the binary to its final location..." -ForegroundColor Cyan
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
    Write-Error "ERROR: Failed to copy the executable to $BinDir: $_"
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# Copy default config if bundled and not already present
$DestConfigToml = Join-Path $ConfigDir "config.toml"
$DestConfigJson = Join-Path $ConfigDir "config.json"
if (-not (Test-Path $DestConfigToml) -and -not (Test-Path $DestConfigJson)) {
    $SrcConfigToml = Join-Path $ExtractDir "config.toml"
    $SrcConfigJson = Join-Path $ExtractDir "config.json"
    if (Test-Path $SrcConfigToml) {
        Write-Host "Copying default config.toml..." -ForegroundColor Cyan
        Copy-Item -Path $SrcConfigToml -Destination $DestConfigToml -Force
    } elseif (Test-Path $SrcConfigJson) {
        Write-Host "Copying default config.json..." -ForegroundColor Cyan
        Copy-Item -Path $SrcConfigJson -Destination $DestConfigJson -Force
    }
}

$ProgressPreference = $oldProgressPreference

Write-Host "$AppName installed successfully at $ExeDestPath!" -ForegroundColor Green

# ── 7. Add to user PATH permanently ────────────────────────────────────────────
$UserPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
$PathList = $UserPath -split ";"
$NormalizedBinDir = $BinDir.TrimEnd('\')

$IsInPath = $false
foreach ($p in $PathList) {
    if ($p.Trim().TrimEnd('\') -eq $NormalizedBinDir) {
        $IsInPath = $true
        break
    }
}

if (-not $IsInPath) {
    Write-Host "Adding $BinDir to your user PATH permanently..." -ForegroundColor Yellow
    $NewUserPath = $UserPath
    if (-not $NewUserPath.EndsWith(";")) {
        $NewUserPath += ";"
    }
    $NewUserPath += $NormalizedBinDir

    [Environment]::SetEnvironmentVariable("Path", $NewUserPath, [EnvironmentVariableTarget]::User)
    $env:Path += ";" + $NormalizedBinDir

    Write-Host "The PATH has been updated. Please restart your terminal for the change to take effect." -ForegroundColor Yellow
} else {
    Write-Host "The directory $BinDir is already in your PATH." -ForegroundColor Green
}

# ── 8. Start Menu shortcut ─────────────────────────────────────────────────────
Write-Host "Creating Start Menu shortcut..." -ForegroundColor Cyan
try {
    $StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    $ShortcutPath = Join-Path $StartMenuDir "Fastty.lnk"

    $WshShell = New-Object -ComObject WScript.Shell
    $Shortcut = $WshShell.CreateShortcut($ShortcutPath)
    $Shortcut.TargetPath = $ExeDestPath
    $Shortcut.WorkingDirectory = $BinDir
    $Shortcut.Description = "Fastty Terminal Emulator"
    $Shortcut.Save()

    Write-Host "Start Menu shortcut created successfully." -ForegroundColor Green
} catch {
    Write-Host "WARNING: Could not create the Start Menu shortcut." -ForegroundColor Yellow
}

# ── 9. Final cleanup ───────────────────────────────────────────────────────────
Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue

# ── 10. Summary ────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host "✓ Binary  -> $BinDir\fastty.exe"
Write-Host "✓ Config  -> $ConfigDir"
Write-Host "✓ Data    -> $DataDir"
Write-Host "✓ State   -> $StateDir"
Write-Host "✓ Cache   -> $CacheDir"
Write-Host ""
Write-Host "Restart your terminal for PATH changes to take effect."
