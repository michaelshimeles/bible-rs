# PR #66 evidence (scratch branch only): drives updates on the runner's desktop, records each leg
# with the evidence-driven-testing recorder when ffmpeg allows, and screenshots every step.
#
# Before: main's app (0.2.0) at before\bible-rs.exe. After: this branch's 0.2.0, installed.
# serve.py offers 0.3.0 as the latest release on 127.0.0.1:8765 and logs requests to server.log.
param([Parameter(Mandatory)][string]$Out, [switch]$Record)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$shell = New-Object -ComObject WScript.Shell
$ev = Join-Path $PSScriptRoot 'evidence.py'
$log = Join-Path $Out 'server.log'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\Bible'
$exe = Join-Path $dir 'bible-rs.exe'
$script:S = $null
$script:failed = 0

function Shot([string]$name) {
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save((Join-Path $Out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "shot $name"
}

function Start-Leg([string]$name, [string]$title) {
    if (-not $Record) { return }
    $started = python $ev start --output (Join-Path $Out $name) --title $title --commit $env:GITHUB_SHA `
        --branch $env:GITHUB_REF_NAME --environment 'GitHub windows-latest runner, 1024x768, Direct3D 12 on WARP'
    Write-Host $started
    $script:S = ($started | ConvertFrom-Json).session
}

function Stop-Leg {
    if ($script:S) { python $ev stop $script:S; $script:S = $null }
}

function Note([string]$type, [string]$message, [string]$result = '') {
    Write-Host "[$type] $message $result"
    if (-not $script:S) { return }
    $a = @($ev, 'annotate', $script:S, '--type', $type, '--message', $message)
    if ($result) { $a += @('--result', $result) }
    python @a | Out-Null
}

function Check([bool]$ok, [string]$message) {
    Note 'assertion' $message $(if ($ok) { 'passed' } else { 'failed' })
    if (-not $ok) { $script:failed++ }
}

function Requests([string]$text) {
    if (-not (Test-Path $log)) { return 0 }
    @(Get-Content $log | Where-Object { $_.Contains($text) }).Count
}

function WaitFor([scriptblock]$cond, [int]$seconds) {
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) {
        if (& $cond) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

function Focus($target) { [void]$shell.AppActivate($target); Start-Sleep -Milliseconds 500 }

function Browsers($since) {
    @(Get-Process msedge, chrome, firefox -ErrorAction SilentlyContinue | Where-Object { $_.StartTime -gt $since })
}

$latest = 'GET /latest.json'
$installer = 'GET /releases/download/v0.3.0/bible-rs-0.3.0-windows-x64-setup.exe'
$env:BIBLE_RS_DEMO = 'size:960x640'

# --- Before: main's update check offers the installer in a message box; Yes opens the browser.
Start-Leg 'before' 'Before: the update check on main'
$legStart = Get-Date
$env:BIBLE_RS_HOME = Join-Path $env:RUNNER_TEMP 'home-before'
Note 'setup' 'main at 0.2.0; a local server offers 0.3.0 as the latest release'
Note 'test_start' 'Before: a newer release is offered through the browser'
$base = Requests $latest
$before = Start-Process (Join-Path $env:GITHUB_WORKSPACE 'before\bible-rs.exe') -PassThru
Check (WaitFor { (Requests $latest) -gt $base } 90) 'The daily check asked for the latest release'
Start-Sleep 4
Shot 'before-01-message-box'
Focus 'A new version of Bible is available'
$shell.SendKeys('~')
Check (WaitFor { (Browsers $legStart).Count -gt 0 } 45) 'Yes opened a web browser'
Start-Sleep 10
Shot 'before-02-browser'
Stop-Leg
Browsers $legStart | Stop-Process -Force -ErrorAction SilentlyContinue
Stop-Process -Id $before.Id -Force -ErrorAction SilentlyContinue
Start-Sleep 2

# --- After: the installed 0.2.0 offers 0.3.0 in the app; Update installs it and reopens.
Start-Leg 'after' 'After: one-click update on Windows (PR #66)'
$legStart = Get-Date
$env:BIBLE_RS_HOME = Join-Path $env:RUNNER_TEMP 'home-after'
Note 'setup' 'Bible 0.2.0 installed; the local server offers 0.3.0 as latest'
Check ((Get-Item $exe).VersionInfo.ProductVersion -like '0.2.0*') 'Precondition: the installed app is 0.2.0'
Note 'test_start' 'It should offer 0.3.0 in the app when Bible starts'
$base = Requests $latest
$old = Start-Process $exe -PassThru
Check (WaitFor { (Requests $latest) -gt $base } 90) 'The daily check asked for the latest release'
Start-Sleep 4
Focus $old.Id
Shot 'after-01-offered'

Note 'test_start' 'It should download, install and reopen 0.3.0 after Update'
$clicked = Get-Date
$shell.SendKeys('~') # Return presses Update
Check (WaitFor { (Requests $installer) -gt 0 } 20) 'Update started the download inside the app'
Start-Sleep 3
Shot 'after-02-downloading'
Start-Sleep 6
Shot 'after-03-downloading'
Check (WaitFor { $old.HasExited } 120) 'Bible 0.2.0 quit so the installer could replace it'
Shot 'after-04-installing'
$script:new = $null
$reopened = WaitFor {
    $script:new = Get-Process bible-rs -ErrorAction SilentlyContinue |
        Where-Object { $_.Id -ne $old.Id -and $_.StartTime -gt $clicked } | Select-Object -First 1
    $null -ne $script:new
} 90
Check $reopened 'The installer opened Bible again'
$version = (Get-Item $exe).VersionInfo.ProductVersion
$registry = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\bible-rs').DisplayVersion
Check ($version -like '0.3.0*' -and $registry -eq '0.3.0') "Installed version is now $version"
if ($script:new) { Check ($script:new.Path -eq $exe) 'The reopened app runs from the install folder' }
Check ((Browsers $legStart).Count -eq 0) 'No web browser opened'
Start-Sleep 8
if ($script:new) { Focus $script:new.Id }
Shot 'after-05-reopened'

Note 'test_start' 'It should say 0.3.0 is the newest version'
$base = Requests $latest
$shell.SendKeys('%h')
Start-Sleep 1
$shell.SendKeys('u')
Check (WaitFor { (Requests $latest) -gt $base } 30) 'Help > Check for Updates asked again'
Start-Sleep 3
Shot 'after-06-up-to-date'
$shell.SendKeys('~')
Start-Sleep 1

Note 'test_start' 'It should explain a failed check in the app, with Retry'
Get-NetTCPConnection -LocalPort 8765 -State Listen -ErrorAction SilentlyContinue |
    ForEach-Object { Stop-Process -Id $_.OwningProcess -Force }
$shell.SendKeys('%h')
Start-Sleep 1
$shell.SendKeys('u')
Start-Sleep 6
Shot 'after-07-offline'
Note 'assertion' 'Error text and Retry: checked by eye in after-07-offline.png' 'untested'
Check ((Browsers $legStart).Count -eq 0) 'Still no web browser opened'
Stop-Leg

Get-Process bible-rs -ErrorAction SilentlyContinue | Stop-Process -Force
Write-Host "failed checks: $script:failed"
exit $script:failed
