# ==============================================================================
# Fasty installation script for Windows (PowerShell)
# ==============================================================================
# Run directly from PowerShell with:
# irm https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.ps1 | iex
# ==============================================================================

$GitHubUser = "diegoleteliers10"
$GitHubRepo = "fasty"
$AppName    = "fasty"
$BinaryName = "fasty.exe"

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

Write-Host "=== Starting $AppName installation on Windows ===" -ForegroundColor Green

# 1. Query the GitHub API for the latest release
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

# 2. Download the Windows .zip asset
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

# 3. Extract
Write-Host "Extracting files..." -ForegroundColor Cyan
try {
    Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir -Force
} catch {
    Write-Error "ERROR: Failed to extract the ZIP archive."
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# 4. Install into the user's local profile directory
$InstallDir = Join-Path $env:USERPROFILE ".local\bin"
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Write-Host "Directory created: $InstallDir" -ForegroundColor Yellow
}

$ExeSourcePath = Join-Path $ExtractDir $BinaryName
$ExeDestPath = Join-Path $InstallDir $BinaryName

Write-Host "Copying the binary to its final location..." -ForegroundColor Cyan
try {
    if (Test-Path $ExeDestPath) {
        # If the executable already exists, rename it to avoid locking issues (file in use)
        $OldExePath = "$ExeDestPath.old"
        if (Test-Path $OldExePath) {
            Remove-Item -Path $OldExePath -Force -ErrorAction SilentlyContinue
        }
        Rename-Item -Path $ExeDestPath -NewName "$BinaryName.old" -Force -ErrorAction SilentlyContinue
    }
    Copy-Item -Path $ExeSourcePath -Destination $ExeDestPath -Force
} catch {
    Write-Error "ERROR: Failed to copy the executable to $InstallDir: $_"
    $ProgressPreference = $oldProgressPreference
    exit 1
}

$ProgressPreference = $oldProgressPreference

Write-Host "$AppName installed successfully at $ExeDestPath!" -ForegroundColor Green

# 5. Add the directory to the user PATH permanently (no administrator privileges)
$UserPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
$PathList = $UserPath -split ";"
$NormalizedInstallDir = $InstallDir.TrimEnd('\')

$IsInPath = $false
foreach ($p in $PathList) {
    if ($p.Trim().TrimEnd('\') -eq $NormalizedInstallDir) {
        $IsInPath = $true
        break
    }
}

if (-not $IsInPath) {
    Write-Host "Adding $InstallDir to your user PATH permanently..." -ForegroundColor Yellow
    $NewUserPath = $UserPath
    if (-not $NewUserPath.EndsWith(";")) {
        $NewUserPath += ";"
    }
    $NewUserPath += $NormalizedInstallDir

    [Environment]::SetEnvironmentVariable("Path", $NewUserPath, [EnvironmentVariableTarget]::User)
    $env:Path += ";" + $NormalizedInstallDir

    Write-Host "The PATH has been updated permanently. Please restart your terminal or code editor for the change to take effect in new sessions." -ForegroundColor Yellow
} else {
    Write-Host "The directory $InstallDir is already in your PATH." -ForegroundColor Green
}

# 6. Create a Start Menu shortcut so it appears as a system application
Write-Host "Creating Start Menu shortcut..." -ForegroundColor Cyan
try {
    $StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    $ShortcutPath = Join-Path $StartMenuDir "Fasty.lnk"

    $WshShell = New-Object -ComObject WScript.Shell
    $Shortcut = $WshShell.CreateShortcut($ShortcutPath)
    $Shortcut.TargetPath = $ExeDestPath
    $Shortcut.WorkingDirectory = $InstallDir
    $Shortcut.Description = "Fasty Terminal Emulator"
    $Shortcut.Save()

    Write-Host "Start Menu shortcut created successfully." -ForegroundColor Green
} catch {
    Write-Host "WARNING: Could not create the Start Menu shortcut." -ForegroundColor Yellow
}

# Final cleanup of the temp directory
Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
