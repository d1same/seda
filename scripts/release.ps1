# Build, sign and publish a Seda release to GitHub (d1same/seda).
#
#   ./scripts/release.ps1 -Version 0.1.3 -Notes notes.md
#   ./scripts/release.ps1 -CleanupOnly
#
# Needs: the update signing key at $env:USERPROFILE\.tauri\seda-updater.key (never commit it; back it
# up somewhere safe: without it, installed copies can't be updated), the GitHub CLI logged in, and the
# version already set in package.json, src-tauri/Cargo.toml and src-tauri/tauri.conf.json.
#
# Publishes the installer plus latest.json, which installed copies of Seda read to find updates.
param(
    [string] $Version,
    [string] $Notes,
    [string] $Repo = "d1same/seda",
    [switch] $Prerelease,   # test releases: not picked up by /releases/latest
    [string] $Tag = "v$Version",
    [string] $Features = "",  # e.g. "test-hooks" for test builds only
    [switch] $CleanupOnly    # only delete leftover folders, then stop
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$full = [IO.Path]::GetFullPath($root).TrimEnd('\')
$onSeda = ($full -eq "Z:\Projects\Whisper\seda") -or ($full -eq "\\10.1.20.4\Ai Projects\Projects\Whisper\seda")
if (-not $onSeda) { throw "Run this from Z:\Projects\Whisper\seda. This copy is $full." }

# Leftovers from earlier copies. The compiler output is Z:\Projects\Whisper\b
# (a longer name breaks the Vulkan build). The installer shelf stays too.
function Remove-OldFolder([string] $dir) {
    if (-not (Test-Path -LiteralPath $dir)) { return }
    cmd /c "attrib -r `"$dir`" /s /d & rmdir /s /q `"$dir`"" | Out-Null
    if (Test-Path -LiteralPath $dir) { Write-Host "Could not remove $dir. Close anything using it and delete that folder by hand." }
    else { Write-Host "Removed old folder $dir" }
}
Remove-OldFolder "C:\seda"
Remove-OldFolder "C:\s"
Remove-OldFolder "Z:\Projects\Whisper\build"
if ($CleanupOnly) { return }
if (-not $Version -or -not $Notes) { throw "Pass -Version and -Notes." }

$key = Join-Path $env:USERPROFILE ".tauri\seda-updater.key"
if (-not (Test-Path $key)) { throw "Signing key not found at $key" }

foreach ($f in "package.json", "src-tauri\tauri.conf.json") {
    if (-not (Select-String -Path (Join-Path $root $f) -Pattern "`"version`": `"$Version`"" -Quiet)) { throw "$f is not at version $Version" }
}

# The Windows build needs the Microsoft linker. A GNU Rust toolchain calls dlltool.exe and stops.
if (-not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
    $vs = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1"
    if (Test-Path -LiteralPath $vs) { & $vs -Arch amd64 }
}
rustup toolchain install stable-x86_64-pc-windows-msvc
if ($LASTEXITCODE) { throw "Microsoft Rust toolchain is not installed" }
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-msvc"

# Signed build. Path remapping keeps this PC's folder names out of the program file.
$env:TAURI_SIGNING_PRIVATE_KEY = $key
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
$env:RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~ --remap-path-prefix=$root=seda"
Push-Location $root
# npm/cargo print progress on stderr; Windows PowerShell would treat that as a failure, so rely on exit codes.
$ErrorActionPreference = "Continue"
try {
    if ($Features) { npm run tauri build -- --features $Features } else { npm run tauri build }
    if ($LASTEXITCODE) { throw "build failed" }
} finally { Pop-Location; $ErrorActionPreference = "Stop" }

$target = (Select-String -Path (Join-Path $root "src-tauri\.cargo\config.toml") -Pattern 'target-dir\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
$bundle = Join-Path $target "release\bundle\nsis"
$installer = Join-Path $bundle "Seda_${Version}_x64-setup.exe"
$sig = [IO.File]::ReadAllText("$installer.sig")

# latest.json: what installed copies check. The signature is verified against the public key in the app.
$latest = [ordered]@{
    version   = $Version
    notes     = [IO.File]::ReadAllText((Resolve-Path $Notes))
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = @{ "windows-x86_64" = @{ signature = $sig.Trim(); url = "https://github.com/$Repo/releases/download/$Tag/Seda_${Version}_x64-setup.exe" } }
}
$latestPath = Join-Path $bundle "latest.json"
[IO.File]::WriteAllText($latestPath, ($latest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))

$ghArgs = @("release", "create", $Tag, $installer, $latestPath, "--repo", $Repo, "--title", "Seda $Version", "--notes-file", $Notes)
if ($Prerelease) { $ghArgs += "--prerelease" }
& gh @ghArgs
if ($LASTEXITCODE) { throw "publishing failed" }
Write-Host "Published $Tag"

# The build above wrote the compiler output to Z:\Projects\Whisper\b. The old drive-root folder can go.
if ($target -ne "Z:/b" -and $target -ne "Z:\b") { Remove-OldFolder "Z:\b" }

# Keep a copy next to the older installers when that folder is on this PC.
$shelf = "Z:\Projects\Whisper\installer"
if (Test-Path -LiteralPath $shelf) {
    Copy-Item -LiteralPath $installer -Destination (Join-Path $shelf "Seda_${Version}_x64-setup.exe") -Force
    Write-Host "Saved installer to $shelf"
}
