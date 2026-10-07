# Fetches a libmpv build for Windows and makes the import library the MSVC
# linker wants (`mpv.lib`), next to `libmpv-2.dll` and the headers.
#
# The builds are shinchiro's, the ones mpv's own site points Windows users
# at. Run from a shell where `lib.exe` and `dumpbin.exe` are on PATH — a
# "Developer PowerShell", or after ilammy/msvc-dev-cmd in CI.

param(
    [string]$Destination = "mpv"
)

$ErrorActionPreference = "Stop"

# Asked without a token, GitHub's API allows 60 requests an hour to an
# address, and CI runners share theirs; with one (GITHUB_TOKEN, which CI
# hands every job) the limit is the token's own.
$headers = @{}
if ($env:GITHUB_TOKEN) { $headers.Authorization = "Bearer $env:GITHUB_TOKEN" }
$release = Invoke-RestMethod -Headers $headers "https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest"
$asset = $release.assets |
    Where-Object { $_.name -match '^mpv-dev-x86_64-\d{8}-git-[0-9a-f]+\.7z$' } |
    Select-Object -First 1
if (-not $asset) { throw "no mpv-dev-x86_64 archive in the latest release" }

New-Item -ItemType Directory -Force $Destination | Out-Null
$archive = Join-Path $env:RUNNER_TEMP ($asset.name)
if (-not $env:RUNNER_TEMP) { $archive = Join-Path ([IO.Path]::GetTempPath()) $asset.name }
Invoke-WebRequest $asset.browser_download_url -OutFile $archive
7z x $archive "-o$Destination" -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw "7z could not unpack $archive" }

# The archive carries the DLL and a MinGW import library; MSVC needs its own,
# made from the DLL's exports.
$dll = Join-Path $Destination "libmpv-2.dll"
if (-not (Test-Path $dll)) { throw "libmpv-2.dll is not in the archive" }
$def = Join-Path $Destination "mpv.def"
$lib = Join-Path $Destination "mpv.lib"

$exports = dumpbin /exports $dll |
    Select-String '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]{8}\s+(\S+)' |
    ForEach-Object { $_.Matches[0].Groups[1].Value }
if (-not $exports) { throw "dumpbin found no exports in $dll" }
@("LIBRARY libmpv-2", "EXPORTS") + $exports | Set-Content -Encoding ascii $def

# Each argument whole, in quotes: a bare `/out:(...)` reaches lib.exe as two
# arguments, and lib then fails on a file it cannot open.
$output = & lib /nologo "/def:$def" /machine:x64 "/out:$lib" 2>&1
if ($LASTEXITCODE -ne 0) {
    $output | Write-Host
    throw "lib.exe could not make $lib"
}
Write-Host "libmpv ready in $Destination ($($exports.Count) exports)"
