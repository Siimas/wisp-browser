# Run a debug build of Wisp on a Windows runner in each layout and save a picture of
# the whole screen for each, so Windows-only appearance can be checked without a
# Windows machine. Used by .github/workflows/windows-shots.yml.
param([string]$Exe = "target\debug\wisp.exe", [string]$Out = "shots")
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Out | Out-Null

Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int ht, bool paint);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
"@
[W]::SetProcessDPIAware() | Out-Null
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Write-Host "screen $($screen.Width)x$($screen.Height)"

function Shot($name) {
  $bmp = New-Object System.Drawing.Bitmap $screen.Width, $screen.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($screen.Location, [System.Drawing.Point]::Empty, $screen.Size)
  $bmp.Save("$Out\$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
}

# name, layout mode, collapsed, theme, script to run in the interface, peek, where to point
$cases = @(
  @("sidebar-right", "sidebar-right", $false, "light", "", $false, ""),
  @("sidebar-left", "sidebar-left", $false, "light", "", $false, ""),
  @("top-relaxed", "top-relaxed", $false, "light", "", $false, ""),
  @("top-compact", "top-compact", $false, "light", "", $false, ""),
  @("sidebar-right-collapsed", "sidebar-right", $true, "light", "", $false, ""),
  @("sidebar-left-collapsed", "sidebar-left", $true, "light", "", $false, ""),
  @("top-relaxed-collapsed", "top-relaxed", $true, "light", "", $false, ""),
  @("top-compact-collapsed", "top-compact", $true, "light", "", $false, ""),
  @("sidebar-right-peek", "sidebar-right", $true, "light", "", $true, ""),
  @("top-relaxed-peek", "top-relaxed", $true, "light", "", $true, ""),
  @("sidebar-right-max", "sidebar-right", $false, "light", "post({cmd:'maximize'})", $false, ""),
  @("top-compact-max", "top-compact", $false, "dark", "post({cmd:'maximize'})", $false, ""),
  @("top-relaxed-collapsed-max", "top-relaxed", $true, "light", "post({cmd:'maximize'})", $false, ""),
  @("hover-close-dark", "sidebar-left", $false, "dark", "", $false, "close"),
  @("hover-maximize", "top-relaxed", $false, "light", "", $false, "max")
)

$data = Join-Path $env:RUNNER_TEMP "wisp-profile"
foreach ($c in $cases) {
  $name, $mode, $collapsed, $theme, $js, $peek, $point = $c
  Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force "$data\Wisp" | Out-Null
  @{ layout = @{ mode = $mode; collapsed = $collapsed }; appearance = @{ theme = $theme } } |
    ConvertTo-Json -Depth 4 | Set-Content "$data\Wisp\config.json"
  $env:APPDATA = $data
  $env:WISP_JS = $js
  $env:WISP_PEEK = $(if ($peek) { "1" } else { "" })
  [W]::SetCursorPos(500, 400) | Out-Null # away from edges that reveal a collapsed bar
  $p = Start-Process -FilePath $Exe -PassThru -NoNewWindow
  Start-Sleep -Seconds 2
  # The runner's screen is small: fit the window, with room around it for its shadow.
  $p.Refresh()
  [W]::MoveWindow($p.MainWindowHandle, 40, 30, 940, 640, $true) | Out-Null
  Start-Sleep -Seconds 7
  if ($point) {
    $r = New-Object W+R
    [W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
    Write-Host "$name window $($r.L),$($r.T) - $($r.Rt),$($r.B)"
    # The window rect includes its invisible side borders (8px): aim at the button centers.
    $x = if ($point -eq "close") { $r.Rt - 31 } else { $r.Rt - 77 }
    [W]::SetCursorPos($x, $r.T + 18) | Out-Null
    Start-Sleep -Seconds 2
  }
  Shot $name
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
  Write-Host "captured $name"
}
