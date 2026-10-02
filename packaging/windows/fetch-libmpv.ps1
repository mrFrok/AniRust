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

$release = Invoke-RestMethod "https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest"
$asset = $release.assets |
    Where-Object { $_.name -match '^mpv-dev-x86_64-\d{8}-git-[0-9a-f]+\.7z$' } |
    Select-Object -First 1
if (-not $asset) { throw "no mpv-dev-x86_64 archive in the latest release" }

New-Item -ItemType Directory -Force $Destination | Out-Null
$archive = Join-Path $env:RUNNER_TEMP ($asset.name)
if (-not $env:RUNNER_TEMP) { $archive = Join-Path ([IO.Path]::GetTempPath()) $asset.name }
Invoke-WebRequest $asset.browser_download_url -OutFile $archive
7z x $archive "-o$Destination" -y | Out-Null

# The archive carries the DLL and a MinGW import library; MSVC needs its own,
# made from the DLL's exports.
$dll = Join-Path $Destination "libmpv-2.dll"
if (-not (Test-Path $dll)) { throw "libmpv-2.dll is not in the archive" }
$def = Join-Path $Destination "mpv.def"
if (-not (Test-Path $def)) {
    $exports = dumpbin /exports $dll |
        Select-String '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]{8}\s+(\S+)' |
        ForEach-Object { $_.Matches[0].Groups[1].Value }
    @("LIBRARY libmpv-2", "EXPORTS") + $exports | Set-Content -Encoding ascii $def
}
lib /nologo /def:$def /machine:x64 /out:(Join-Path $Destination "mpv.lib") | Out-Null
Write-Host "libmpv ready in $Destination"
