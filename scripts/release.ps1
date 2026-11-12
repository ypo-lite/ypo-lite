<#
.SYNOPSIS
  Builds the workspace in release mode, signs the executables, builds the
  Inno Setup installer and signs it, and packs the portable zip. Output in dist\:
  ypo-lite-setup-<version>.exe, ypo-lite-<version>-x64.zip, SHA256SUMS.txt

.PARAMETER Tag
  The release tag being built, e.g. release/0.2.0. Stops here if it does not
  match the workspace version. Release tags are immutable, so .github\workflows\
  release.yml passes it to catch a mismatch before anything is published.

.PARAMETER CertThumbprint
  SHA-1 thumbprint of a code-signing certificate in the user/machine store.

.PARAMETER SignArgs
  Raw signtool arguments instead of a thumbprint, e.g. for Azure Trusted Signing:
  '/dlib "...\Azure.CodeSigning.Dlib.dll" /dmdf metadata.json'. Wins over -CertThumbprint.

  Without either option, everything is built unsigned.

.PARAMETER BuildOnly
  Stops after cargo build. With -SkipBuild, this lets the workflow compile before
  the signing key is on the machine, out of reach of the build scripts.

.PARAMETER SkipBuild
  Signs and packages the executables already in target\release.
#>
param(
    [string]$Tag,
    [string]$CertThumbprint,
    [string]$SignArgs,
    [string]$TimestampUrl = 'http://timestamp.digicert.com',
    [switch]$BuildOnly,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$apps = 'y-snap', 'y-snap-config', 'y-pop', 'y-pop-config', 'y-coustic', 'y-coustic-config'

# Workspace version, single source of truth for the installer.
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value

if ($Tag -and $Tag -ne "release/$version") {
    throw "tag $Tag does not match the workspace version $version"
}

$signing = $CertThumbprint -or $SignArgs
$signtoolArgs = if ($SignArgs) { $SignArgs } else { "/sha1 $CertThumbprint" }
$signtoolArgs = "sign /fd sha256 /tr $TimestampUrl /td sha256 $signtoolArgs"

function Find-Tool($name, $candidates) {
    $found = Get-Command $name -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    foreach ($pattern in $candidates) {
        $hit = Get-ChildItem $pattern -ErrorAction SilentlyContinue | Sort-Object FullName | Select-Object -Last 1
        if ($hit) { return $hit.FullName }
    }
    throw "$name not found: install it or add it to PATH."
}

if (-not $BuildOnly) {
    $iscc = Find-Tool 'ISCC.exe' @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe")

    if ($signing) {
        $signtool = Find-Tool 'signtool.exe' @(
            "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe")
    }
}

if (-not $SkipBuild) {
    Write-Host "== cargo build --release ($version)"
    $packages = $apps | ForEach-Object { '-p'; $_ }
    cargo build --release --locked @packages
    if ($LASTEXITCODE) { throw 'cargo build failed' }
}

if ($BuildOnly) { return }

if ($signing) {
    Write-Host '== signing executables'
    foreach ($app in $apps) {
        & $signtool sign /fd sha256 /tr $TimestampUrl /td sha256 $(if ($SignArgs) { $SignArgs -split ' ' } else { '/sha1', $CertThumbprint }) "target\release\$app.exe"
        if ($LASTEXITCODE) { throw "signing $app failed" }
    }
}

Write-Host '== building installer'
$isccArgs = @("/DAppVersion=$version")
if ($signing) {
    # $q is Inno Setup's own quote: a real one inside an argument does not survive
    # the way PowerShell passes arguments to a program.
    $isccArgs += '/DSign=1', "/Ssigntool=`$q$signtool`$q $signtoolArgs `$f"
}
& $iscc @isccArgs installer\ypo-lite.iss
if ($LASTEXITCODE) { throw 'ISCC failed' }

Write-Host '== packing the portable zip'
# The configurations go next to the executables, which makes the copy portable.
$setup = "dist\ypo-lite-setup-$version.exe"
$zip = "dist\ypo-lite-$version-x64.zip"
$contents = @($apps | ForEach-Object { "target\release\$_.exe" }) +
    'y-snap\rsc\y-snap.xml', 'y-pop\rsc\y-pop.xml', 'y-coustic\rsc\y-coustic.xml', 'LICENSE'
Compress-Archive -Path $contents -DestinationPath $zip -Force

Write-Host '== checksums'
Get-FileHash $setup, $zip -Algorithm SHA256 |
    ForEach-Object { '{0}  {1}' -f $_.Hash.ToLowerInvariant(), (Split-Path -Leaf $_.Path) } |
    Set-Content dist\SHA256SUMS.txt -Encoding ascii

Write-Host "== done: $setup, $zip, dist\SHA256SUMS.txt"
