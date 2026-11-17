# Check a Windows release with Microsoft Defender before it is published, on a
# throwaway GitHub Windows runner. Exits non-zero (and the release stops) if Defender
# flags anything or a check could not be trusted.
#
#   pwsh .github/defender-check.ps1 -Exe <wisp.exe> -Installer <Setup.exe> `
#        [-Launcher <debug wisp.exe built with a throwaway update key> -Key <that key>]
#
# 1. Update rehearsal (with -Launcher): with real-time protection on, a copy of Wisp
#    updates itself to this installer the way installed copies will (download,
#    signature check, installer started on quit, Wisp restarted), and the installed
#    Wisp is started again, twice. This is where Defender flagged 0.2.1
#    (Trojan:Win32/Bearfoos.A!ml) while scans of the same file were clean, so it runs
#    first, before anything else has opened the new files. The verdict is not
#    deterministic: the release runs this on several fresh runners at once.
# 2. Scan: wisp.exe and the installer, with fresh definitions and cloud lookups on
#    (samples never sent).
# Both phases first prove Defender works on the harmless EICAR test file.
#
# Protection is only ever turned up here: the runner image ships with real-time
# protection off and C:\ and D:\ excluded; this removes the exclusions and turns it on.
# See docs/WINDOWS-SECURITY.md.
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Installer,
    [string]$Launcher,
    [string]$Key,
    [int]$Rounds = 2
)
$ErrorActionPreference = 'Stop'
$mp = "$env:ProgramFiles\Windows Defender\MpCmdRun.exe"
$Exe = (Resolve-Path $Exe).Path
$Installer = (Resolve-Path $Installer).Path
$expected = (Get-FileHash $Exe).Hash
# (Built from two halves so this script itself is not an antivirus test file.)
$eicarText = 'X5O!P%@AP[4\PZX54(P^)7CC)7}$EICAR' + '-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*'
# Everything Defender has found on this runner, except our own EICAR controls.
function Get-Threats { Get-MpThreat -ErrorAction SilentlyContinue | Where-Object ThreatName -notmatch 'EICAR' | ForEach-Object ThreatName }

$status = Get-MpComputerStatus
if (-not ($status.AMServiceEnabled -and $status.AntivirusEnabled)) {
    throw "Microsoft Defender is not running on this runner: the check would prove nothing."
}
# Cloud lookups on, at the strictest level, potentially unwanted apps blocked; samples
# are never uploaded.
Set-MpPreference -MAPSReporting Advanced -SubmitSamplesConsent NeverSend -CloudBlockLevel High -PUAProtection Enabled
& $mp -SignatureUpdate
if ($LASTEXITCODE -ne 0) { throw "Defender could not update its definitions (exit $LASTEXITCODE)." }
$status = Get-MpComputerStatus
"Defender engine $($status.AMEngineVersion), definitions $($status.AntivirusSignatureVersion) of $($status.AntivirusSignatureLastUpdated)"
$summary = @("### Microsoft Defender", "", "Definitions $($status.AntivirusSignatureVersion)", "")
$failed = @()

# Control: the scanner must detect EICAR. (Done first, while real-time protection is
# still off, or the file would be removed before it could be scanned.)
$dir = Join-Path $env:RUNNER_TEMP 'defender-scan'
New-Item -ItemType Directory -Force $dir | Out-Null
$eicar = Join-Path $dir 'eicar-scan-control.com'
Set-Content $eicar $eicarText -NoNewline
& $mp -Scan -ScanType 3 -File $eicar -DisableRemediation | Out-Null
if ($LASTEXITCODE -ne 2) { throw "Defender did not detect the EICAR test file (exit $LASTEXITCODE): the scan cannot be trusted." }
Remove-Item $eicar
"Control: the scanner detected the EICAR test file."

# --- 1. Update rehearsal --------------------------------------------------------------
if ($Launcher) {
    Remove-MpPreference -ExclusionPath 'C:\', 'D:\' -ErrorAction SilentlyContinue
    Set-MpPreference -DisableRealtimeMonitoring $false -DisableBehaviorMonitoring $false -DisableIOAVProtection $false
    $pref = Get-MpPreference
    if ($pref.ExclusionPath -or $pref.ExclusionProcess -or $pref.ExclusionExtension) {
        throw "Defender still has exclusions ($($pref.ExclusionPath) $($pref.ExclusionProcess) $($pref.ExclusionExtension)): the rehearsal would prove nothing."
    }
    foreach ($i in 1..30) { if ((Get-MpComputerStatus).RealTimeProtectionEnabled) { break }; Start-Sleep 2 }
    if (-not (Get-MpComputerStatus).RealTimeProtectionEnabled) { throw "Real-time protection could not be turned on: the rehearsal would prove nothing." }
    # Control: with real-time protection working, opening an EICAR file in the user's
    # local app data fails with "the file contains a virus" (ERROR_VIRUS_INFECTED, the
    # error the owner saw for 0.2.1). This runner's Defender scans when a file is opened.
    # A new file each attempt: protection takes a moment to start, and a file opened
    # before that is remembered as clean.
    New-Item -ItemType Directory -Force (Join-Path $env:LOCALAPPDATA 'defender-control') | Out-Null
    $caught = $false
    foreach ($i in 1..30) {
        $ctl = Join-Path $env:LOCALAPPDATA "defender-control\eicar-realtime-control-$i.com"
        try {
            Set-Content $ctl $eicarText -NoNewline -ErrorAction Stop
            Start-Sleep 1
            $null = [IO.File]::ReadAllBytes($ctl)
        } catch {
            if ($_.Exception.InnerException.HResult -eq 0x800700E1 -or $_.Exception.HResult -eq 0x800700E1 -or "$_" -match 'virus') { $caught = $true; break }
        }
        Start-Sleep 1
    }
    if (-not $caught) { throw "Real-time protection did not block the EICAR test file: the rehearsal would prove nothing." }
    "Control: real-time protection blocked the EICAR test file."

    # A local update server offering this installer, signed with the throwaway key the
    # launcher was built with (the release key never leaves its own step).
    $srv = Join-Path $env:RUNNER_TEMP 'update-server'
    New-Item -ItemType Directory -Force $srv | Out-Null
    Copy-Item $Installer (Join-Path $srv 'Setup.exe')
    cargo packager signer sign -k $Key --password '' (Join-Path $srv 'Setup.exe')
    if ($LASTEXITCODE -ne 0) { throw "could not sign the rehearsal installer" }
    $sig = (Get-Content (Join-Path $srv 'Setup.exe.sig') -Raw).Trim()
    @{ version = '9.9.9'; platforms = @{ 'windows-x86_64' = @{ url = 'http://127.0.0.1:8765/Setup.exe'; signature = $sig; format = 'nsis' } } } |
        ConvertTo-Json -Depth 5 | Set-Content (Join-Path $srv 'latest.json')
    $server = Start-Process python -ArgumentList '-m', 'http.server', '8765', '--bind', '127.0.0.1', '--directory', $srv -WindowStyle Hidden -PassThru
    Start-Sleep 3
    $env:WISP_UPDATE_ENDPOINT = 'http://127.0.0.1:8765/latest.json'
    # The installed copy that updates itself lives outside the build folder, as on a PC.
    $copy = 'C:\wisp-rehearsal\wisp.exe'
    New-Item -ItemType Directory -Force (Split-Path $copy) | Out-Null
    Copy-Item $Launcher $copy

    $installed = Join-Path $env:LOCALAPPDATA 'Wisp\wisp.exe'
    foreach ($round in 1..$Rounds) {
        # Every round starts with nothing installed, so an earlier round cannot pass it.
        Get-Process wisp -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep 2
        Remove-Item (Split-Path $installed) -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path $installed) { throw "could not remove the previous round's install" }
        # Wisp checks 5 s after starting, downloads and verifies, and installs on quit.
        $p = Start-Process $copy -PassThru
        Start-Sleep 45
        taskkill /PID $p.Id | Out-Null   # a polite close, like the window's close button
        if ($LASTEXITCODE -ne 0) { $failed += "rehearsal round ${round}: could not close the rehearsal copy"; break }
        Start-Sleep 60                   # the installer runs, then starts Wisp again (/R)
        Get-Process wisp -ErrorAction SilentlyContinue | Stop-Process
        Start-Sleep 3
        # Start the updated Wisp as a person would. Nothing has read the new file yet.
        $starts = foreach ($n in 1, 2) {
            try { Start-Process $installed -ErrorAction Stop | Out-Null; Start-Sleep 20; Get-Process wisp -ErrorAction SilentlyContinue | Stop-Process; Start-Sleep 3; 'ok' }
            catch { "BLOCKED ($($_.Exception.Message))" }
        }
        $now = (Get-FileHash $installed -ErrorAction SilentlyContinue).Hash
        "Rehearsal round ${round}: started: $($starts -join ', '); installed wisp.exe is this build: $($now -eq $expected)"
        if ($starts -ne 'ok') { $failed += "rehearsal round ${round}: Wisp could not start after the update" }
        if ($now -ne $expected) { $failed += "rehearsal round ${round}: the update did not leave this wisp.exe installed" }
        if ($failed -or (Get-Threats)) { break }
    }
    Stop-Process $server -ErrorAction SilentlyContinue
    $summary += "Update rehearsal: $round of $Rounds round(s) with real-time protection on.", ""
}

# --- 2. Scan --------------------------------------------------------------------------
$summary += "| File | SHA-256 | Scan |", "| --- | --- | --- |"
foreach ($file in $Exe, $Installer) {
    $copy = Join-Path $dir (Split-Path $file -Leaf)
    Copy-Item $file $copy
    $hash = (Get-FileHash $copy -Algorithm SHA256).Hash.ToLower()
    $out = & $mp -Scan -ScanType 3 -File $copy -DisableRemediation
    $code = $LASTEXITCODE
    $out | Where-Object { $_ -match 'Threat\s*:' } | ForEach-Object { "  $_" }
    $result = switch ($code) { 0 { 'clean' } 2 { 'FLAGGED' } default { "scan failed (exit $code)" } }
    if ($code -ne 0) { $failed += "scan of $(Split-Path $file -Leaf): $result" }
    "$result  $hash  $(Split-Path $file -Leaf)"
    $summary += "| $(Split-Path $file -Leaf) | ``$hash`` | $result |"
}

$names = Get-Threats
if ($names) {
    $failed += "Defender detected: $($names -join ', ')"
    Get-MpThreatDetection | Format-List InitialDetectionTime, ProcessName, Resources
}
if ($env:GITHUB_STEP_SUMMARY) { ($summary + '' + ($failed | ForEach-Object { "- **$_**" })) | Add-Content $env:GITHUB_STEP_SUMMARY }
if ($failed) {
    $failed | ForEach-Object { "::error::$_" }
    throw "Microsoft Defender check failed: not publishing."
}
"Microsoft Defender check passed."
exit 0
