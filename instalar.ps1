# ==============================================================================
# Script de Instalación de PowerShell para Windows (Fasty)
# ==============================================================================
# Ejecutar directamente desde PowerShell con:
# irm https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.ps1 | iex
# ==============================================================================

# CONFIGURACIÓN
$GitHubUser = "diegoleteliers10"
$GitHubRepo = "fasty"
$AppName    = "fasty"
$BinaryName = "fasty.exe"

# Habilitar TLS 1.2 para conexiones HTTPS
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

Write-Host "=== Iniciando instalación de $AppName en Windows ===" -ForegroundColor Green

# 1. Consultar la API de GitHub para obtener la última versión
Write-Host "🔍 Consultando la última versión en la API de GitHub..." -ForegroundColor Cyan
$ApiUrl = "https://api.github.com/repos/$GitHubUser/$GitHubRepo/releases/latest"

try {
    $oldProgressPreference = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'
    
    $Response = Invoke-RestMethod -Uri $ApiUrl -Method Get
    $LatestTag = $Response.tag_name
} catch {
    Write-Error "❌ Error: No se pudo obtener la última versión desde la API de GitHub. Verifica la conexión o el repositorio."
    $ProgressPreference = $oldProgressPreference
    exit 1
}

Write-Host "📦 Última versión encontrada: $LatestTag" -ForegroundColor Green

# 2. Descargar el archivo .zip de Windows
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

Write-Host "📥 Descargando $ZipName..." -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -UseBasicParsing
} catch {
    Write-Error "❌ Error al descargar el archivo desde $DownloadUrl"
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# 3. Descomprimir archivos
Write-Host "🔓 Descomprimiendo archivos..." -ForegroundColor Cyan
try {
    Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir -Force
} catch {
    Write-Error "❌ Error al descomprimir el archivo ZIP."
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# 4. Instalar en el directorio del perfil local del usuario
$InstallDir = Join-Path $env:USERPROFILE ".local\bin"
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Write-Host "📂 Directorio creado: $InstallDir" -ForegroundColor Yellow
}

$ExeSourcePath = Join-Path $ExtractDir $BinaryName
$ExeDestPath = Join-Path $InstallDir $BinaryName

Write-Host "🚀 Copiando binario a su ubicación final..." -ForegroundColor Cyan
try {
    Copy-Item -Path $ExeSourcePath -Destination $ExeDestPath -Force
} catch {
    Write-Error "❌ Error al copiar el archivo ejecutable a $InstallDir"
    $ProgressPreference = $oldProgressPreference
    exit 1
}

# Restablecer la preferencia de progreso original
$ProgressPreference = $oldProgressPreference

Write-Host "🎉 ¡$AppName instalado con éxito en $ExeDestPath!" -ForegroundColor Green

# 5. Agregar la ruta al PATH del usuario de manera permanente (sin privilegios de administrador)
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
    Write-Host "⚙️ Añadiendo $InstallDir al PATH de tu usuario de forma permanente..." -ForegroundColor Yellow
    $NewUserPath = $UserPath
    if (-not $NewUserPath.EndsWith(";")) {
        $NewUserPath += ";"
    }
    $NewUserPath += $NormalizedInstallDir
    
    [Environment]::SetEnvironmentVariable("Path", $NewUserPath, [EnvironmentVariableTarget]::User)
    $env:Path += ";" + $NormalizedInstallDir
    
    Write-Host "💡 El PATH se ha actualizado permanentemente. Por favor, reinicia tu terminal o editor de código para aplicar los cambios en nuevas sesiones." -ForegroundColor Yellow
} else {
    Write-Host "✅ El directorio $InstallDir ya se encuentra en tu PATH." -ForegroundColor Green
}

# 6. Crear un acceso directo en el Menú Inicio para que aparezca como aplicación del sistema
Write-Host "🖥️ Creando acceso directo en el Menú de Inicio..." -ForegroundColor Cyan
try {
    $StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    $ShortcutPath = Join-Path $StartMenuDir "Fasty.lnk"
    
    $WshShell = New-Object -ComObject WScript.Shell
    $Shortcut = $WshShell.CreateShortcut($ShortcutPath)
    $Shortcut.TargetPath = $ExeDestPath
    $Shortcut.WorkingDirectory = $InstallDir
    $Shortcut.Description = "Fasty Terminal Emulator"
    $Shortcut.Save()
    
    Write-Host "✅ Acceso directo creado en el Menú Inicio con éxito." -ForegroundColor Green
} catch {
    Write-Host "⚠️ Advertencia: No se pudo crear el acceso directo en el Menú Inicio." -ForegroundColor Yellow
}

# Limpieza final del directorio temporal
Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
