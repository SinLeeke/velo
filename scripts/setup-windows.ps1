# Prepara un PC Windows 10/11 nuevo para desarrollar Velo.
# Ejecutar en PowerShell COMO ADMINISTRADOR:
#   powershell -ExecutionPolicy Bypass -File scripts\setup-windows.ps1
# Tarda 20-40 minutos y descarga ~8-10 GB. Las instalaciones van una por una (winget falla si se lanzan en paralelo).

$ErrorActionPreference = "Stop"
$QtVersion = "6.10.3"
$QtRoot = "C:\Qt"
$QtBin = "$QtRoot\$QtVersion\msvc2022_64\bin"

function Install-Winget($id, $extra = @()) {
    Write-Host "== $id" -ForegroundColor Cyan
    winget install -e --id $id --accept-source-agreements --accept-package-agreements @extra
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne -1978335189) { # -1978335189 = ya instalado
        Write-Warning "$id terminó con código $LASTEXITCODE"
    }
}

# 1. Compilador C++ de Microsoft (MSVC v143 + Windows SDK)
Install-Winget "Microsoft.VisualStudio.2022.BuildTools" @("--override", "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended")
# 2. Rust, CMake, Ninja, Python (para aqtinstall), Git
Install-Winget "Rustlang.Rustup"
Install-Winget "Kitware.CMake"
Install-Winget "Ninja-build.Ninja"
Install-Winget "Python.Python.3.13"
Install-Winget "Git.Git"

# Refrescar PATH en esta sesión
$env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [Environment]::GetEnvironmentVariable("Path", "User") + ";$env:USERPROFILE\.cargo\bin"

# 3. Toolchain de Rust
rustup default stable-x86_64-pc-windows-msvc
rustup component add clippy rustfmt

# 4. Qt 6.10.3 (MSVC 2022 64 bits + qtshadertools) sin cuenta de Qt
python -m pip install --user --upgrade aqtinstall
python -m aqt install-qt windows desktop $QtVersion win64_msvc2022_64 -m qtshadertools -O $QtRoot
[Environment]::SetEnvironmentVariable("QMAKE", "$QtBin\qmake.exe", "User")
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$QtBin*") { [Environment]::SetEnvironmentVariable("Path", "$userPath;$QtBin", "User") }

Write-Host ""
Write-Host "Listo. Cierra y vuelve a abrir la terminal, y luego:" -ForegroundColor Green
Write-Host "  1. cd <carpeta del proyecto>"
Write-Host "  2. cargo test --workspace   (la primera compilación tarda ~10 min)"
Write-Host "  3. cargo run -p velo"
