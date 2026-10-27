# Publish a Seda release. 0.2.10 is already on GitHub. Write the real change
# in notes.md, then paste this for the next release:
#
#   cd Z:\Projects\Whisper\seda
#   Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
#   .\scripts\release.ps1 -Version 0.2.11 -Notes notes.md
#
# When the window says to expect a password, press Enter. The password is empty.
# The installer is published to GitHub and copied to Z:\Projects\Whisper\installer.
# README.md lists the errors from the 0.2.10 publish and what to do for each one.
#
# -Version is written into package.json, src-tauri/tauri.conf.json, and
# src-tauri/Cargo.toml. The signing key stays at
# %USERPROFILE%\.tauri\seda-updater.key.
# If the installer is already built, add -SkipBuild to sign and publish that file.
param(
    [string] $Version,
    [string] $Notes,
    [string] $Repo = "d1same/seda",
    [switch] $Prerelease,   # test releases: not picked up by /releases/latest
    [string] $Tag = "v$Version",
    [string] $Features = "",  # e.g. "test-hooks" for test builds only
    [switch] $CleanupOnly,   # only delete leftover folders, then stop
    [switch] $SkipBuild      # sign and publish the installer that is already built
)
function Set-AppVersion([string] $Root, [string] $Version) {
    $utf8 = New-Object System.Text.UTF8Encoding $false
    foreach ($rel in @("package.json", "src-tauri\tauri.conf.json")) {
        $path = Join-Path $Root $rel
        $text = [IO.File]::ReadAllText($path)
        $match = [regex]::Match($text, '"version"\s*:\s*"([^"]+)"')
        if (-not $match.Success) { throw "$rel has no version." }
        $old = $match.Groups[1].Value
        if ($old -eq $Version) { continue }
        $span = $match.Groups[1]
        $next = $text.Remove($span.Index, $span.Length).Insert($span.Index, $Version)
        [IO.File]::WriteAllText($path, $next, $utf8)
        Write-Host "Set $rel to $Version."
    }
    $cargoPath = Join-Path $Root "src-tauri\Cargo.toml"
    $cargo = [IO.File]::ReadAllText($cargoPath)
    $match = [regex]::Match($cargo, '(?m)^version = "([^"]+)"')
    if (-not $match.Success) { throw "src-tauri/Cargo.toml has no version." }
    $old = $match.Groups[1].Value
    if ($old -ne $Version) {
        $span = $match.Groups[1]
        $next = $cargo.Remove($span.Index, $span.Length).Insert($span.Index, $Version)
        [IO.File]::WriteAllText($cargoPath, $next, $utf8)
        Write-Host "Set src-tauri/Cargo.toml to $Version."
    }
}

function Read-ReleaseNotes([string] $NotesPath) {
    $noteText = [IO.File]::ReadAllText((Resolve-Path -LiteralPath $NotesPath)).Trim()
    $examples = @(
        "What changed, in one or two sentences.",
        "Say what changed in this release."
    )
    if ($examples -contains $noteText) {
        throw "notes.md still has the example sentence. Write what changed, then run this again."
    }
    if (-not $noteText) { throw "notes.md is empty. Write what changed, then run this again." }
    return $noteText
}

function Test-ReleasePublished([string] $Repo, [string] $Tag) {
    gh release view $Tag --repo $Repo --json tagName 1>$null 2>$null
    return ($LASTEXITCODE -eq 0)
}

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

function Remove-BuildTree([string] $dir) {
    if (-not (Test-Path -LiteralPath $dir)) { return }
    if (Get-Command cmd.exe -ErrorAction SilentlyContinue) {
        cmd /c "attrib -r `"$dir`" /s /d & rmdir /s /q `"$dir`"" | Out-Null
    }
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }
}

# Whisper marks its shader steps as safe to run together. On Windows they share
# one folder, so configure, build, and install start at once and the folder vanishes.
function Repair-VulkanShaderSteps([string] $TargetDir) {
    $utf8 = New-Object System.Text.UTF8Encoding $false
    $lists = @()
    $cargoSrc = Join-Path $env:USERPROFILE ".cargo\registry\src"
    if (Test-Path -LiteralPath $cargoSrc) {
        $lists += @(Get-ChildItem -Path (Join-Path $cargoSrc "*\whisper-rs-sys-*\whisper.cpp\ggml\src\ggml-vulkan\CMakeLists.txt") -File -ErrorAction SilentlyContinue)
    }
    if ($TargetDir -and (Test-Path -LiteralPath $TargetDir)) {
        $lists += @(Get-ChildItem -Path (Join-Path $TargetDir "release\build\whisper-rs-sys-*\out\whisper.cpp\ggml\src\ggml-vulkan\CMakeLists.txt") -File -ErrorAction SilentlyContinue)
        foreach ($prefix in @(Get-ChildItem -Path (Join-Path $TargetDir "release\build\whisper-rs-sys-*\out\build\ggml\src\ggml-vulkan\vulkan-shaders-gen-prefix") -Directory -ErrorAction SilentlyContinue)) {
            Remove-BuildTree $prefix.FullName
        }
    }
    $changed = $false
    $prefixLine = '        PREFIX "$ENV{SEDA_SHADER_DIR}"'
    foreach ($file in $lists) {
        if (-not $file) { continue }
        $text = [IO.File]::ReadAllText($file.FullName)
        $next = $text.Replace("cmake_policy(SET CMP0147 NEW)", "cmake_policy(SET CMP0147 OLD)")
        if (-not $next.Contains($prefixLine)) {
            $crlf = "vulkan-shaders-gen`r`n        SOURCE_DIR"
            $lf = "vulkan-shaders-gen`n        SOURCE_DIR"
            if ($next.Contains($crlf)) {
                $next = $next.Replace($crlf, "vulkan-shaders-gen`r`n$prefixLine`r`n        SOURCE_DIR")
            } elseif ($next.Contains($lf)) {
                $next = $next.Replace($lf, "vulkan-shaders-gen`n$prefixLine`n        SOURCE_DIR")
            }
        }
        if ($next -eq $text) { continue }
        [IO.File]::WriteAllText($file.FullName, $next, $utf8)
        $changed = $true
    }
    if ($changed) { Write-Host "The shader tool now builds in its own short folder and waits its turn." }
}

# Compiler files sit next to the repo, inside Z:\Projects\Whisper or Z:\Projects\Seda.
function Get-SedaLayout([string] $RepoRoot) {
    $given = $RepoRoot.TrimEnd('\', '/')
    $full = $given
    if (-not [string]::IsNullOrWhiteSpace($RepoRoot)) {
        try { $full = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\', '/') } catch { }
    }
    $allowed = @(
        "Z:\Projects\Whisper\seda",
        "\\10.1.20.4\Ai Projects\Projects\Whisper\seda",
        "Z:\Projects\Seda\seda",
        "\\10.1.20.4\Ai Projects\Projects\Seda\seda"
    )
    $root = $null
    foreach ($candidate in @($full, $given)) {
        if ($allowed -contains $candidate) { $root = $candidate; break }
    }
    if (-not $root) {
        return [pscustomobject]@{ Root = $full; Allowed = $false; Target = $null; Shader = $null; Shelf = $null }
    }
    $parent = Split-Path $root -Parent
    if ($root -match '\\') { $parent = ($parent -replace '/', '\') }
    [pscustomobject]@{
        Root = $root
        Allowed = $true
        Target = Join-ProjectPath $parent "b"
        Shader = Join-ProjectPath $parent "shader"
        Shelf = Join-ProjectPath $parent "installer"
    }
}

function Join-ProjectPath([string] $Parent, [string] $Child) {
    if ($Parent -match '\\') { return ($Parent.TrimEnd('\') + '\' + $Child) }
    return ($Parent.TrimEnd('/') + '/' + $Child)
}

# After a rename, an old compiler cache still names the other folder.
function Clear-RenamedFolderCaches([string] $TargetDir, [string] $RepoRoot) {
    if (-not $TargetDir -or -not (Test-Path -LiteralPath $TargetDir)) { return }
    $parentName = Split-Path (Split-Path $RepoRoot -Parent) -Leaf
    $other = if ($parentName -eq "Seda") { "Whisper" } else { "Seda" }
    $caches = @(Get-ChildItem -Path (Join-Path $TargetDir "release\build\whisper-rs-sys-*\out\build\CMakeCache.txt") -File -ErrorAction SilentlyContinue)
    foreach ($cache in $caches) {
        if (-not $cache) { continue }
        $text = [IO.File]::ReadAllText($cache.FullName)
        if ($text -notmatch "Projects[\\/]$other") { continue }
        Write-Host "Removing a compiler cache that still points at the $other folder."
        Remove-BuildTree (Split-Path $cache.FullName -Parent)
    }
}

function Get-TauriCli([string] $Root) {
    foreach ($name in @("tauri.js", "main.js")) {
        $path = Join-Path $Root "node_modules\@tauri-apps\cli\$name"
        if (Test-Path -LiteralPath $path) { return $path }
    }
    throw "Tauri's command was not found. Run npm install in the project folder."
}

function Get-SignArguments([string] $Cli, [string] $KeyPath, [string] $Version, [string] $Installer) {
    @($Cli, "signer", "sign", "--private-key-path", $KeyPath, "--app-version", $Version, $Installer)
}

# Run in this window. The password question has to be able to hear Enter.
# Do not pass an empty -p. Windows PowerShell drops it, and Tauri then stops.
function Invoke-SignInstaller([string] $Root, [string] $KeyPath, [string] $Version, [string] $Installer) {
    $cli = Get-TauriCli $Root
    Write-Host "Press Enter at the password question. The password is empty."
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
    $signArgs = @(Get-SignArguments $cli $KeyPath $Version $Installer)
    & node @signArgs
    if ($LASTEXITCODE) { throw "signing failed" }
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
$layout = Get-SedaLayout $root
if (-not $layout.Allowed) { throw "Run this from Z:\Projects\Whisper\seda. This copy is $($layout.Root)." }

# Leftovers from earlier copies. Compiler output stays next to the repo.
function Remove-OldFolder([string] $dir) {
    if (-not (Test-Path -LiteralPath $dir)) { return }
    Write-Host "Removing $dir"
    cmd /c "attrib -r `"$dir`" /s /d & rmdir /s /q `"$dir`"" | Out-Null
    if (Test-Path -LiteralPath $dir) { Write-Host "Could not remove $dir. Close anything using it and delete that folder by hand." }
    else { Write-Host "Removed $dir" }
}
if ($SkipBuild) { Write-Host "Signing the installer that is already built." }
else {
    Remove-OldFolder "C:\seda"
    Remove-OldFolder "C:\s"
    Remove-OldFolder "Z:\b"
    Remove-OldFolder "Z:\Projects\Whisper\build"
}
if ($CleanupOnly) { return }
if (-not $Version -or -not $Notes) { throw "Pass -Version and -Notes." }
$noteText = Read-ReleaseNotes $Notes

$key = Join-Path $env:USERPROFILE ".tauri\seda-updater.key"
if (-not (Test-Path $key)) { throw "Signing key not found at $key" }
if (Test-ReleasePublished $Repo $Tag) { throw "$Tag is already published. Pass the next version." }

Set-AppVersion $root $Version

$target = $layout.Target
$bundle = Join-Path $target "release\bundle\nsis"
$installer = Join-Path $bundle "Seda_${Version}_x64-setup.exe"

# The Windows build needs the Microsoft linker. A GNU Rust toolchain calls dlltool.exe and stops.
if (-not $SkipBuild -and -not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
    $vs = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1"
    if (Test-Path -LiteralPath $vs) { & $vs -Arch amd64 }
}
if (-not $SkipBuild) {
    rustup toolchain install stable-x86_64-pc-windows-msvc
    if ($LASTEXITCODE) { throw "Microsoft Rust toolchain is not installed" }
    $env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-msvc"
}

# Path remapping keeps this PC's folder names out of the program file.
$env:RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~ --remap-path-prefix=$root=seda"
# The shader tool builds in the shader folder next to the repo. Tracking stays off so a deep
# test compile does not trip MSBuild's file tracker.
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

if (-not $SkipBuild) {
$shader = $layout.Shader -replace '\\', '/'
$env:CARGO_TARGET_DIR = $target -replace '\\', '/'
$env:CARGO_BUILD_TARGET_DIR = $env:CARGO_TARGET_DIR
# Dev builds read shader-dev from .cargo/config.toml. This release build keeps shader.
$env:SEDA_SHADER_DIR = $shader
if (-not (Test-Path -LiteralPath $target)) { Write-Host "Creating $target for the compiler." }
New-Item -ItemType Directory -Force -Path $target | Out-Null
if (-not (Test-Path -LiteralPath $shader)) { Write-Host "Creating $shader for the shader tool." }
New-Item -ItemType Directory -Force -Path $shader | Out-Null
# Imported by every MSBuild project under the compiler folder. Custom steps wait
# their turn, and the file tracker stays off.
$props = @"
<Project>
  <PropertyGroup>
    <TrackFileAccess>false</TrackFileAccess>
  </PropertyGroup>
  <Target Name="SedaSerialCustomBuild" BeforeTargets="CustomBuild">
    <ItemGroup>
      <CustomBuild>
        <BuildInParallel>false</BuildInParallel>
      </CustomBuild>
    </ItemGroup>
  </Target>
</Project>
"@
$props = $props.Trim() + "`r`n"
$utf8 = New-Object System.Text.UTF8Encoding $false
[IO.File]::WriteAllText((Join-Path $target "Directory.Build.targets"), $props, $utf8)
Clear-RenamedFolderCaches $target $root
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
}
if (-not $SkipBuild) {
    Write-Host "Building the installer. This takes a while. The password question comes after it."
    Repair-VulkanShaderSteps $target
    $log = Join-Path $target "release-build.log"
    $npm = "npm run tauri build -- --no-sign"
    if ($Features) { $npm = "npm run tauri build -- --features $Features --no-sign" }
    Push-Location $root
    # npm/cargo print progress on stderr; Windows PowerShell would treat that as a failure, so rely on exit codes.
    # --no-sign keeps the password question out of this logged build. Signing happens in this window afterward.
    $ErrorActionPreference = "Continue"
    try {
        $code = Invoke-Logged $root $log $npm
        if ($code -ne 0) {
            Get-BuildErrorLines $log | ForEach-Object { Write-Host $_ }
            throw "build failed"
        }
    } finally { Pop-Location; $ErrorActionPreference = "Stop" }
}
if (-not (Test-Path -LiteralPath $installer)) { throw "Installer was not found at $installer" }
Invoke-SignInstaller $root $key $Version $installer
$sig = [IO.File]::ReadAllText("$installer.sig")

# latest.json: what installed copies check. The signature is verified against the public key in the app.
$latest = [ordered]@{
    version   = $Version
    notes     = $noteText
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = @{ "windows-x86_64" = @{ signature = $sig.Trim(); url = "https://github.com/$Repo/releases/download/$Tag/Seda_${Version}_x64-setup.exe" } }
}
$latestPath = Join-Path $bundle "latest.json"
$utf8 = New-Object Text.UTF8Encoding $false
[IO.File]::WriteAllText($latestPath, ($latest | ConvertTo-Json -Depth 5), $utf8)
$notesFile = Join-Path $bundle "release-notes.txt"
[IO.File]::WriteAllText($notesFile, $noteText, $utf8)

Write-Host "Publishing $Tag."
$ghArgs = @("release", "create", $Tag, $installer, $latestPath, "--repo", $Repo, "--title", "Seda $Version", "--notes-file", $notesFile)
if ($Prerelease) { $ghArgs += "--prerelease" }
& gh @ghArgs
if ($LASTEXITCODE) { throw "publishing failed" }
Write-Host "Published $Tag"

# Keep a copy next to the older installers when that folder is on this PC.
$shelf = $layout.Shelf
if (Test-Path -LiteralPath $shelf) {
    Copy-Item -LiteralPath $installer -Destination (Join-Path $shelf "Seda_${Version}_x64-setup.exe") -Force
    Write-Host "Saved installer to $shelf"
}
