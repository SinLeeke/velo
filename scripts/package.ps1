# Genera un paquete portable con la app pública compartida; no lee archivos locales.
[CmdletBinding()]
param(
    [string]$QtBin = 'C:\Qt\6.10.3\msvc2022_64\bin',
    # Opcional: un release ya enlazado. No cambia el Client ID de ese ejecutable.
    [string]$ExecutablePath = '',
    # Opcional: generar también el instalador si Inno Setup 6 está disponible.
    [switch]$Installer
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$distDir = Join-Path $repoRoot 'dist'
$stageDir = Join-Path $distDir '.Velo-staging'
$packageDir = Join-Path $distDir 'Velo'
$zipPath = Join-Path $distDir 'Velo-win64.zip'
$deployQt = Join-Path $QtBin 'windeployqt.exe'

# Windows PowerShell 5 convierte stderr de herramientas nativas en ErrorRecord al
# redirigir un log. Una advertencia no es un fallo: manda el código de salida.
function Invoke-CheckedNative([string]$Program, [string[]]$Arguments) {
    $resolvedProgram = (Get-Command -Name $Program -CommandType Application -ErrorAction Stop).Source
    $ErrorActionPreference = 'Continue'
    & $resolvedProgram @Arguments
    if ($LASTEXITCODE -ne 0) { throw 'Falló una herramienta de compilación/despliegue; no se generó un paquete nuevo.' }
}

# Nunca borrar o mover un destino calculado sin comprobar su ruta absoluta primero.
function Assert-DistPath([string]$Path) {
    $resolved = [IO.Path]::GetFullPath($Path)
    $boundary = [IO.Path]::GetFullPath($distDir) + [IO.Path]::DirectorySeparatorChar
    if (!$resolved.StartsWith($boundary, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'El destino calculado no está dentro de dist.'
    }
    # No seguir enlaces/junctions al limpiar una distribución anterior.
    foreach ($candidate in @($distDir, $resolved)) {
        if ((Test-Path -LiteralPath $candidate) -and
            ((Get-Item -LiteralPath $candidate -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'dist y sus destinos no pueden ser enlaces o junctions.'
        }
    }
}

function Assert-CleanPackage([string]$Folder) {
    $files = @(Get-ChildItem -LiteralPath $Folder -File -Recurse -Force)
    foreach ($file in $files) {
        # Solo ejecutable, dependencias Qt/MSVC y documentos elegidos explícitamente.
        if ($file.Name -match '(?i)(\.env$|\.pdb$|token|credential|settings\.toml|ui\.toml)' -or
            $file.Extension -match '(?i)^\.(toml|log|db|sqlite|key|pem)$') {
            throw 'El paquete contiene un archivo de configuración o credenciales; no se publicará.'
        }
        if ($file.Extension -match '(?i)^\.(qml|js|json|txt|md|conf|ini)$') {
            $content = [IO.File]::ReadAllText($file.FullName)
            if ($content -match '(?i)(access_token|refresh_token|client_secret)["\s]*[:=]\s*["\x27]?[A-Za-z0-9._~-]{12,}' -or
                $content -match 'eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+') {
                throw 'Se encontró material de autenticación en el paquete; no se publicará.'
            }
        }
    }
}

$oldPath = $env:Path
$oldQmake = $env:QMAKE
$oldVcInstall = $env:VCINSTALLDIR
$oldClientId = $env:VELO_SPOTIFY_CLIENT_ID
try {
    if (!(Test-Path -LiteralPath $deployQt)) { throw 'No se encontró windeployqt; instala Qt 6.10.3 MSVC o indica -QtBin.' }
    $env:Path = "$env:USERPROFILE\.cargo\bin;$QtBin;$oldPath"
    $env:QMAKE = Join-Path $QtBin 'qmake.exe'
    # Un paquete distribuible siempre compila el predeterminado compartido.
    # Los overrides de desarrollo quedan fuera del build y se restauran al terminar.
    $env:VELO_SPOTIFY_CLIENT_ID = $null
    Push-Location $repoRoot
    try {
        Write-Host 'Compilando Velo release…'
        if ($ExecutablePath) {
            $exePath = [IO.Path]::GetFullPath($ExecutablePath)
            Write-Host 'Usando el release indicado: debe haber sido compilado para distribución con la app compartida.'
        } else {
            Invoke-CheckedNative 'cargo' @('build', '--release', '-p', 'velo')
            $metadataJson = Invoke-CheckedNative 'cargo' @('metadata', '--offline', '--format-version', '1', '--no-deps')
            $targetDir = ($metadataJson | ConvertFrom-Json).target_directory
            $exePath = Join-Path $targetDir 'release\velo.exe'
        }
        if (!(Test-Path -LiteralPath $exePath)) { throw 'No se encontró velo.exe release.' }
        foreach ($path in @($stageDir, $packageDir, $zipPath)) { Assert-DistPath $path }
        New-Item -ItemType Directory -Path $distDir -Force | Out-Null
        if (Test-Path -LiteralPath $stageDir) { Remove-Item -LiteralPath $stageDir -Recurse -Force }
        New-Item -ItemType Directory -Path $stageDir | Out-Null
        Copy-Item -LiteralPath $exePath -Destination (Join-Path $stageDir 'velo.exe')
        Write-Host 'Incluyendo Qt y sus módulos QML…'
        # Solo para windeployqt: VCINSTALLDIR sin el PATH completo de una consola
        # MSVC hace que rustc omita su detección de herramientas y tome link.exe de Git.
        if (!$env:VCINSTALLDIR) {
            $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
            if (Test-Path -LiteralPath $vswhere) {
                $visualStudio = Invoke-CheckedNative $vswhere @('-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath')
                if ($visualStudio) { $env:VCINSTALLDIR = Join-Path $visualStudio 'VC' }
            }
        }
        Invoke-CheckedNative $deployQt @('--release', '--qmldir', 'qml', '--compiler-runtime', (Join-Path $stageDir 'velo.exe'))
        # Qt 6.10 puede desplegar vc_redist.x64.exe en vez de DLL app-local. El amigo
        # debe poder abrir Velo sin instalarlo: copiar el CRT redistribuible de MSVC.
        if ($env:VCINSTALLDIR) {
            $redistRoot = Join-Path $env:VCINSTALLDIR 'Redist\MSVC'
            if (Test-Path -LiteralPath $redistRoot) {
                $crtDir = Get-ChildItem -LiteralPath $redistRoot -Directory |
                    Where-Object { $_.Name -match '^\d' } | Sort-Object Name -Descending |
                    ForEach-Object {
                        $x64 = Join-Path $_.FullName 'x64'
                        if (Test-Path -LiteralPath $x64) { Get-ChildItem -LiteralPath $x64 -Directory -Filter 'Microsoft.VC*.CRT' }
                    } | Select-Object -First 1
                if ($crtDir) {
                    Get-ChildItem -LiteralPath $crtDir.FullName -File -Filter '*.dll' |
                        ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $stageDir }
                }
            }
        }
        foreach ($runtime in @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')) {
            if (!(Test-Path -LiteralPath (Join-Path $stageDir $runtime))) {
                throw 'Falta el runtime MSVC. Ejecuta desde Developer PowerShell de Visual Studio con VCINSTALLDIR definido.'
            }
        }
        $runtimeInstaller = Join-Path $stageDir 'vc_redist.x64.exe'
        if (Test-Path -LiteralPath $runtimeInstaller) { Remove-Item -LiteralPath $runtimeInstaller -Force }
        # qml/fonts e icons son recursos del ejecutable (crates/app/ui/build.rs).
        # Sus licencias también deben viajar aunque los assets no sean archivos externos.
        Copy-Item -LiteralPath (Join-Path $repoRoot 'THIRD_PARTY_NOTICES.md') -Destination $stageDir
        Copy-Item -LiteralPath (Join-Path $repoRoot 'qml\icons\LICENSE') -Destination (Join-Path $stageDir 'LICENSE-icons.txt')
        foreach ($fontLicense in @(Get-ChildItem -LiteralPath (Join-Path $repoRoot 'qml\fonts') -File | Where-Object { $_.Name -match 'OFL|LICENSE' })) {
            Copy-Item -LiteralPath $fontLicense.FullName -Destination (Join-Path $stageDir ('LICENSE-font-' + $fontLicense.BaseName + '.txt'))
        }
        $readme = @'
Velo — tu música en este PC

1. Extrae todo el zip. Abre la carpeta Velo y haz doble clic en velo.exe.
   Conserva las DLL y carpetas junto al ejecutable. No hace falta instalar Qt.
2. Si Windows SmartScreen muestra una advertencia para esta app sin firma:
   comprueba que recibiste el zip de tu amigo; luego «Más información → Ejecutar de todas formas».
3. Pulsa «Iniciar sesión con Spotify». En el navegador puedes entrar con tu
   correo, Google, Apple o Facebook. No necesitas conocer ningún Client ID.
4. Al volver a la biblioteca, pulsa «Activar la reproducción en este PC».
   Spotify pedirá una autorización de audio una sola vez. Necesitas Spotify Premium.
5. Elige una canción. Tus sesiones se guardan en el Administrador de credenciales
   de Windows; al cerrar sesión Velo olvida ambas autorizaciones.

No hace falta que tu amigo agregue tu cuenta: Velo usa una app pública compartida.
Spotify puede limitar su cuota o desactivarla. Si sucede, abre Ajustes → Cuenta →
Usar mi propia app de Spotify y sigue el asistente. La dirección exacta es
http://127.0.0.1:8989/login; marca Web API y pega sólo el Client ID.
Si se abre Spotify con otra cuenta en el navegador, cámbiala antes de autorizar.

En Ajustes → Apariencia puedes activar Modo rendimiento o Reducir movimiento.
El enlace «Usar mi propia app de Spotify» también está en el inicio de sesión.
Para salir: pulsa tu avatar/nombre → Cerrar sesión (o en Ajustes → Cuenta).
'@
        [IO.File]::WriteAllText((Join-Path $stageDir 'LEEME.txt'), $readme, [Text.UTF8Encoding]::new($false))
        Assert-CleanPackage $stageDir
        if (Test-Path -LiteralPath $packageDir) { Remove-Item -LiteralPath $packageDir -Recurse -Force }
        Move-Item -LiteralPath $stageDir -Destination $packageDir
        Compress-Archive -LiteralPath $packageDir -DestinationPath $zipPath -Force
        # Inspeccionar también las entradas del zip que realmente se va a enviar.
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $archive = [IO.Compression.ZipFile]::OpenRead($zipPath)
        try {
            foreach ($entry in $archive.Entries) {
                if ($entry.FullName -match '(?i)(velo\.local\.env|token|credential|\.toml$|\.pdb$)') {
                    throw 'El zip contiene un archivo local o credenciales.'
                }
            }
        } finally { $archive.Dispose() }
        Write-Host 'Listo: dist/Velo-win64.zip (sin archivo local, configuración ni tokens).'
        if ($Installer) {
            $isccCommand = Get-Command 'ISCC.exe' -CommandType Application -ErrorAction SilentlyContinue
            $isccPath = if ($isccCommand) { $isccCommand.Source } else { $null }
            foreach ($installRoot in @($env:LOCALAPPDATA, ${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
                if ($isccPath) { break }
                if (!$installRoot) { continue }
                $relativePath = if ($installRoot -eq $env:LOCALAPPDATA) { 'Programs\Inno Setup 6\ISCC.exe' } else { 'Inno Setup 6\ISCC.exe' }
                $candidate = Join-Path $installRoot $relativePath
                if (Test-Path -LiteralPath $candidate -PathType Leaf -ErrorAction SilentlyContinue) { $isccPath = $candidate }
            }
            if ($isccPath) {
                Invoke-CheckedNative $isccPath @((Join-Path $repoRoot 'installer\velo.iss'))
                if (!(Test-Path -LiteralPath (Join-Path $distDir 'Velo-Setup-0.9.1.exe'))) {
                    throw 'ISCC terminó sin generar dist/Velo-Setup-0.9.1.exe.'
                }
                Write-Host 'Listo: dist/Velo-Setup-0.9.1.exe.'
            } else {
                Write-Warning 'No se encontró un ISCC.exe accesible (Inno Setup 6); el zip portable está listo, pero no se generó el instalador.'
            }
        }
    } finally { Pop-Location }
} finally {
    $env:Path = $oldPath
    $env:QMAKE = $oldQmake
    $env:VCINSTALLDIR = $oldVcInstall
    $env:VELO_SPOTIFY_CLIENT_ID = $oldClientId
}
