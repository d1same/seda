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
function Get-BuildErrorLines([string] $LogPath) {
    $header = "Build failed. Full log: $LogPath"
    if (-not (Test-Path -LiteralPath $LogPath)) {
        return @($header, "The build log was not written.")
    }
    $lines = @(Get-Content -LiteralPath $LogPath -ErrorAction SilentlyContinue)
    if ($lines.Count -eq 0) {
        return @($header, "The build log is empty.")
    }
    $noise = '(?i)(CMake Warning \(dev\)|Policy CMP\d+|This warning is for project developers|Use -Wno-dev|cmake_policy command|For compatibility with older versions of CMake|Since the policy is not set|legacy libraries de-duplication|compiler extensions won''t be enabled|Selection of language standard|De-duplicate libraries on link lines|Called from:)'
    $hit = '(?i)(CMake Error|build script failed|FTK1011|error MSB\d+|error C\d+|error LNK\d+|fatal error|LINK : fatal|ninja: error|Command line error|unresolved external symbol|glslc: error|: error|error:)'
    $errors = New-Object System.Collections.Generic.List[string]
    $kept = New-Object System.Collections.Generic.List[string]
    foreach ($line in $lines) {
        if ($line -match $noise) { continue }
        $kept.Add($line)
        if ($line -match $hit) { $errors.Add($line) }
    }
    if ($errors.Count -gt 0) {
        $out = New-Object System.Collections.Generic.List[string]
        $out.Add($header)
        $out.Add("These lines are the actual errors. The CMake policy text above them is only a warning.")
        $take = $errors.Count
        if ($take -gt 40) { $take = 40 }
        foreach ($line in $errors.GetRange($errors.Count - $take, $take)) { $out.Add($line) }
        return $out.ToArray()
    }
    $out = New-Object System.Collections.Generic.List[string]
    $out.Add($header)
    $out.Add("No compiler error line was found. The CMake policy text is only a warning. Last lines:")
    $take = $kept.Count
    if ($take -gt 50) { $take = 50 }
    if ($take -eq 0) { $out.Add("(nothing left after hiding the policy warnings)") }
    else { foreach ($line in $kept.GetRange($kept.Count - $take, $take)) { $out.Add($line) } }
    return $out.ToArray()
}

function Find-BrokenWhisperBuilds([string] $TargetDir) {
    $hits = New-Object System.Collections.Generic.List[string]
    $crateRoot = Join-Path $TargetDir "release\build"
    if (-not (Test-Path -LiteralPath $crateRoot)) { return }
    Get-ChildItem -LiteralPath $crateRoot -Directory -Filter "whisper-rs-sys-*" | ForEach-Object {
        $cache = Join-Path $_.FullName "out\build\CMakeCache.txt"
        if (-not (Test-Path -LiteralPath $cache)) { return }
        if (Select-String -LiteralPath $cache -Pattern 'CMAKE_CXX_FLAGS[^=]*=/FS /DWIN32' -Quiet) {
            $hits.Add($_.FullName)
        }
    }
    if ($hits.Count -eq 0) { return }
    return $hits.ToArray()
}

function Invoke-Logged([string] $WorkDir, [string] $LogPath, [string] $CommandLine, [string] $Shell = "cmd.exe", [string] $ShellArgs = "") {
    $utf8 = New-Object System.Text.UTF8Encoding $false
    $writer = New-Object System.IO.StreamWriter($LogPath, $false, $utf8)
    $writer.AutoFlush = $true
    $queue = New-Object System.Collections.Concurrent.ConcurrentQueue[string]
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Shell
    if (-not $ShellArgs) { $ShellArgs = "/d /s /c `"$CommandLine`"" }
    $psi.Arguments = $ShellArgs
    $psi.WorkingDirectory = $WorkDir
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    $action = {
        if ($null -ne $EventArgs.Data) { [void]$Event.MessageData.Enqueue($EventArgs.Data) }
    }
    $outEvent = $null
    $errEvent = $null
    try {
        $outEvent = Register-ObjectEvent -InputObject $proc -EventName OutputDataReceived -Action $action -MessageData $queue
        $errEvent = Register-ObjectEvent -InputObject $proc -EventName ErrorDataReceived -Action $action -MessageData $queue
        [void]$proc.Start()
        $proc.BeginOutputReadLine()
        $proc.BeginErrorReadLine()
        while (-not $proc.HasExited) {
            $text = $null
            while ($queue.TryDequeue([ref]$text)) {
                Write-Host $text
                $writer.WriteLine($text)
            }
            Start-Sleep -Milliseconds 100
        }
        $proc.WaitForExit()
        $quiet = 0
        while ($quiet -lt 5) {
            $text = $null
            $got = $false
            while ($queue.TryDequeue([ref]$text)) {
                $got = $true
                Write-Host $text
                $writer.WriteLine($text)
            }
            if ($got) { $quiet = 0 } else { $quiet++; Start-Sleep -Milliseconds 100 }
        }
        $code = $proc.ExitCode
        return $code
    } finally {
        if ($outEvent) { Unregister-Event -SourceIdentifier $outEvent.Name -ErrorAction SilentlyContinue }
        if ($errEvent) { Unregister-Event -SourceIdentifier $errEvent.Name -ErrorAction SilentlyContinue }
        $writer.Dispose()
        $proc.Dispose()
    }
}

# Dot-sourcing loads the helpers above without publishing a release.
if ($MyInvocation.InvocationName -eq '.') { return }

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
# The Vulkan shader try-compile path is still too long for MSBuild's file tracker.
# Do not set CMAKE_C_FLAGS or CMAKE_CXX_FLAGS here. whisper-rs-sys copies those into
# cmake, and cmake then drops its own flags, including /utf-8. /FS rides in CL instead.
foreach ($name in @("CMAKE_C_FLAGS", "CMAKE_CXX_FLAGS")) {
    $cur = [Environment]::GetEnvironmentVariable($name, "Process")
    if ($cur -and $cur -match '/FS /DWIN32') { Remove-Item "Env:$name" }
}
foreach ($name in @("CL", "_CL_")) {
    $cur = [Environment]::GetEnvironmentVariable($name, "Process")
    if ([string]::IsNullOrWhiteSpace($cur)) { Set-Item "Env:$name" "/FS" }
    elseif ($cur -notmatch '(^|\s)/FS(\s|$)') { Set-Item "Env:$name" ($cur.Trim() + " /FS") }
}
$env:TrackFileAccess = "false"
$env:MSBUILDDISABLENODEREUSE = "1"
$env:CMAKE_POLICY_DEFAULT_CMP0128 = "OLD"
$env:CMAKE_POLICY_DEFAULT_CMP0156 = "OLD"

$target = (Select-String -Path (Join-Path $root "src-tauri\.cargo\config.toml") -Pattern 'target-dir\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
New-Item -ItemType Directory -Force -Path $target | Out-Null
# Imported late by every MSBuild project under the compiler folder, including the
# nested Vulkan shader try-compile, so the file tracker stays off.
$props = "<Project>`r`n  <PropertyGroup>`r`n    <TrackFileAccess>false</TrackFileAccess>`r`n  </PropertyGroup>`r`n</Project>`r`n"
$utf8 = New-Object System.Text.UTF8Encoding $false
[IO.File]::WriteAllText((Join-Path $target "Directory.Build.targets"), $props, $utf8)
foreach ($dir in @(Find-BrokenWhisperBuilds $target)) {
    Write-Host "Removing a half-finished Whisper build that used the broken compiler flags."
    cmd /c "attrib -r `"$dir`" /s /d & rmdir /s /q `"$dir`"" | Out-Null
    if (Test-Path -LiteralPath $dir) { Write-Host "Could not remove $dir. Close anything using it and delete that folder by hand." }
    $fp = Join-Path (Join-Path $target "release\.fingerprint") (Split-Path $dir -Leaf)
    if (Test-Path -LiteralPath $fp) {
        cmd /c "attrib -r `"$fp`" /s /d & rmdir /s /q `"$fp`"" | Out-Null
        if (Test-Path -LiteralPath $fp) { Write-Host "Could not remove $fp. Close anything using it and delete that folder by hand." }
    }
}
$log = Join-Path $target "release-build.log"
$npm = "npm run tauri build"
if ($Features) { $npm += " -- --features $Features" }
Push-Location $root
# npm/cargo print progress on stderr; Windows PowerShell would treat that as a failure, so rely on exit codes.
$ErrorActionPreference = "Continue"
try {
    $code = Invoke-Logged $root $log $npm
    if ($code -ne 0) {
        Get-BuildErrorLines $log | ForEach-Object { Write-Host $_ }
        throw "build failed"
    }
} finally { Pop-Location; $ErrorActionPreference = "Stop" }

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
