# Checks locales de la frontera de archivos y del rechazo de credenciales, sin compilar.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$distDir = Join-Path $repoRoot 'dist'
$fixture = Join-Path $distDir '.package-test'
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'package.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'package.ps1 no se puede analizar.' }
# Cargar solo las funciones, sin ejecutar el build ni leer el archivo local.
foreach ($name in @('Assert-DistPath', 'Assert-CleanPackage', 'Invoke-CheckedNative')) {
    $definition = $ast.Find({ param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true)
    if (!$definition) { throw 'Falta una función de empaquetado.' }
    . ([ScriptBlock]::Create($definition.Extent.Text))
}
function Expect-Rejection([ScriptBlock]$Action, [string]$Name) {
    $rejected = $false
    try { & $Action } catch { $rejected = $true }
    if (!$rejected) { throw "No se rechazó: $Name" }
}

Assert-DistPath $fixture
Expect-Rejection { Assert-DistPath (Join-Path $repoRoot 'qml') } 'ruta fuera de dist'
Expect-Rejection { Assert-DistPath (Join-Path $distDir '..\qml') } 'escape con ..'
New-Item -ItemType Directory -Path $fixture -Force | Out-Null
try {
    $safe = Join-Path $fixture 'LEEME.txt'
    [IO.File]::WriteAllText($safe, 'Archivo de prueba sin credenciales.')
    Assert-CleanPackage $fixture
    $local = Join-Path $fixture 'velo.local.env'
    [IO.File]::WriteAllText($local, 'DATO_DE_PRUEBA=ninguno')
    Expect-Rejection { Assert-CleanPackage $fixture } 'archivo local'
    Remove-Item -LiteralPath $local
    [IO.File]::WriteAllText($safe, '{"refresh_token":"valor-sintetico-sin-sesion"}')
    Expect-Rejection { Assert-CleanPackage $fixture } 'token en documento'
    [IO.File]::WriteAllText($safe, 'eyJtest.eyJtest.invalid-signature')
    Expect-Rejection { Assert-CleanPackage $fixture } 'JWT en documento'
    [IO.File]::WriteAllText($safe, 'Archivo limpio.')
    Assert-CleanPackage $fixture
    # stderr con exit 0 no debe fallar bajo Windows PowerShell 5 ni contaminar stdout.
    $nativeOut = Invoke-CheckedNative 'powershell.exe' @('-NoProfile', '-Command', "[Console]::Error.WriteLine('advertencia sintetica'); Write-Output 'ok'; exit 0")
    if ($nativeOut -ne 'ok') { throw 'stderr contaminó la salida estructurada.' }
    Expect-Rejection { Invoke-CheckedNative 'powershell.exe' @('-NoProfile', '-Command', 'exit 7') } 'código de salida no cero'
    Expect-Rejection { Invoke-CheckedNative 'velo-nonexistent-tool.exe' @() } 'herramienta inexistente'
    Write-Host 'package.tests: sintaxis, rutas, rechazo de archivos/tokens y códigos nativos OK.'
} finally {
    Assert-DistPath $fixture
    Remove-Item -LiteralPath $fixture -Recurse -Force
}
