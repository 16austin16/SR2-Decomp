# Caller survey, longer exploration: like run_callers.ps1, but plays a varied sequence
# (leave the crib, sprint, jump, attack, weapons, phone, map, pause tabs; -Mode vehicle
# also tries to get into and drive a car) so more classes run. Takes about 10 minutes.
# Build first:
#   cargo build --release --target i686-pc-windows-msvc -p sr2-hook --features caller-survey
# Restore the normal DLL afterwards (rebuild without the feature and redeploy).
param([string]$Label = "explore", [string]$Mode = "foot")
$g = "E:\SteamGames\steamapps\common\Saints Row 2"
$bin = "F:\Saints Row 2 Decomp\bin"
$in = "$PSScriptRoot\sr2input.ps1"
function Alive { [bool](Get-Process SR2_pc -ErrorAction SilentlyContinue) }
function K($code, $ms = 0) { if (Alive) { if ($ms) { & $in hold $code -HoldMs $ms | Out-Null } else { & $in key $code | Out-Null } } }
if (Alive) { "game already running"; exit 1 }
Copy-Item "F:\Saints Row 2 Decomp\target\i686-pc-windows-msvc\release\dinput8.dll" "$g\dinput8.dll" -Force
Remove-Item "$g\sr2-hook.log", "$g\sr2-callers.log" -ErrorAction SilentlyContinue
Start-Process "steam://rungameid/9480"
$log = "$g\sr2-hook.log"
$deadline = (Get-Date).AddSeconds(150)
while ((Get-Date) -lt $deadline -and -not ((Test-Path $log) -and (Select-String -Path $log -Pattern "find_entry verify" -Quiet))) {
    if ((Test-Path $log) -and -not (Alive)) { "GAME EXITED during boot"; break }
    Start-Sleep 2
}
if (Alive) {
    foreach ($i in 1..5) { K "1C"; K "01" }
    $deadline = (Get-Date).AddSeconds(240)
    while ((Get-Date) -lt $deadline -and -not (Select-String -Path $log -Pattern "frame 3600" -Quiet)) {
        if (-not (Alive)) { "GAME EXITED during load"; break }
        Start-Sleep 3
    }
    & $in shot "$bin\$Label.loaded.png" | Out-Null
    if ($Mode -eq "teleport" -or $Mode -eq "vehicle") {
        # Needs TELEPORT_TEST in the build: wait for the hook's 8 jumps through the city
        # (the player ends outside, 600 m from the crib); input would restart the test.
        $deadline = (Get-Date).AddSeconds(480)
        while ((Get-Date) -lt $deadline -and -not (Select-String -Path $log -Pattern "teleport test: done" -Quiet)) {
            if (-not (Alive)) { "GAME EXITED during teleports"; break }
            Start-Sleep 5
        }
    } else {
        # Leave the crib: walk, open doors (E), keep walking.
        foreach ($s in @(@("11", 5000), @("12", 0), @("11", 4000), @("12", 0), @("1E", 1500), @("11", 5000), @("12", 0), @("11", 5000))) { K $s[0] $s[1] }
    }
    & $in shot "$bin\$Label.outside.png" | Out-Null
    # On foot: sprint, jump, crouch, attack, weapons, reload, look around.
    foreach ($r in 1..3) {
        K "2A,11" 4000; K "39"; K "1D"; K "1D"
        if (Alive) { & $in click "640,360" | Out-Null; & $in click "640,360" | Out-Null; & $in rclick "640,360" | Out-Null }
        foreach ($w in "02", "03", "04", "05") { K $w; if (Alive) { & $in click "640,360" | Out-Null } }
        K "13"; K "10"; K "22"; K "0F"
        if (Alive) { & $in move "300,0" | Out-Null; & $in move "-200,50" | Out-Null; & $in wheel "120" | Out-Null }
        K "20" 2000; K "11" 3000
    }
    if ($Mode -eq "vehicle") {
        # Try to take a car: walk around pressing the enter-vehicle keys, then drive.
        foreach ($r in 1..6) {
            K "11" 3000; K "21"; K "12"; K "21"; Start-Sleep 3
            K "11" 4000; K "1E" 1000; K "11" 3000; K "20" 1000; K "39"; K "2A,11" 2000
        }
        & $in shot "$bin\$Label.vehicle.png" | Out-Null
    }
    # Phone, map and the pause menu tabs.
    K "E048"; Start-Sleep 2; K "E050"; K "1C"; Start-Sleep 2; K "01"; K "01"
    K "32"; Start-Sleep 3; K "01"
    K "01"; Start-Sleep 2
    foreach ($i in 1..8) { K "1D"; Start-Sleep 1; K "1C"; Start-Sleep 1; K "01"; Start-Sleep 1 }
    & $in shot "$bin\$Label.menu.png" | Out-Null
    K "01"; Start-Sleep 2
    # Let the survey thread write the log twice more.
    Start-Sleep 65
    & $in shot "$bin\$Label.end.png" | Out-Null
    Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
    Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
}
Copy-Item $log "$bin\$Label.hook.log" -ErrorAction SilentlyContinue
Copy-Item "$g\sr2-callers.log" "$bin\$Label.callers.log" -ErrorAction SilentlyContinue
Get-Content $log | Select-String -Pattern "callers:|panic|error|failed|CRASH" | ForEach-Object { $_.Line.Substring(0, [Math]::Min(220, $_.Line.Length)) }
if (Test-Path "$bin\$Label.callers.log") { "callers.log lines: " + (Get-Content "$bin\$Label.callers.log").Count } else { "no callers.log" }
