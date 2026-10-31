# Caller survey run: deploy the caller-survey build of the hook DLL, boot, Continue
# into the save, then walk around and open the pause menu tabs so more classes run.
# Waits for two sr2-callers.log writes, closes the game and copies the logs to
# bin/<label>.*. Build first:
#   cargo build --release --target i686-pc-windows-msvc -p sr2-hook --features caller-survey
# Restore the normal DLL afterwards (rebuild without the feature and redeploy).
param([string]$Label = "callers")
$g = "E:\SteamGames\steamapps\common\Saints Row 2"
$bin = "F:\Saints Row 2 Decomp\bin"
$input = "$PSScriptRoot\sr2input.ps1"
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) { "game already running"; exit 1 }
Copy-Item "F:\Saints Row 2 Decomp\target\i686-pc-windows-msvc\release\dinput8.dll" "$g\dinput8.dll" -Force
Remove-Item "$g\sr2-hook.log", "$g\sr2-callers.log" -ErrorAction SilentlyContinue
Start-Process "steam://rungameid/9480"
$log = "$g\sr2-hook.log"
$deadline = (Get-Date).AddSeconds(150)
while ((Get-Date) -lt $deadline -and -not ((Test-Path $log) -and (Select-String -Path $log -Pattern "find_entry verify" -Quiet))) {
    if ((Test-Path $log) -and -not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED during boot"; break }
    Start-Sleep 2
}
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) {
    foreach ($i in 1..5) { & $input key "1C" | Out-Null; & $input key "01" | Out-Null }
    $deadline = (Get-Date).AddSeconds(240)
    while ((Get-Date) -lt $deadline -and -not (Select-String -Path $log -Pattern "frame 3600" -Quiet)) {
        if (-not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED during load"; break }
        Start-Sleep 3
    }
    & $input shot "$bin\$Label.loaded.png" | Out-Null
    # Walk forward and turn a few times (W held, then A/D).
    foreach ($k in "11", "1E", "11", "20", "11") {
        if (-not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { break }
        & $input key $k -HoldMs 4000 | Out-Null
    }
    # Pause menu: open, step through the tabs, close.
    & $input key "01" | Out-Null; Start-Sleep 2
    foreach ($i in 1..6) { & $input key "1D" | Out-Null; Start-Sleep 1 }
    & $input shot "$bin\$Label.menu.png" | Out-Null
    & $input key "01" | Out-Null; Start-Sleep 2
    # Let the survey thread write the log twice more.
    Start-Sleep 65
    & $input shot "$bin\$Label.end.png" | Out-Null
    Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
    Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
}
Copy-Item $log "$bin\$Label.hook.log" -ErrorAction SilentlyContinue
Copy-Item "$g\sr2-callers.log" "$bin\$Label.callers.log" -ErrorAction SilentlyContinue
Get-Content $log | Select-String -Pattern "callers:|panic|error|failed" | ForEach-Object { $_.Line.Substring(0, [Math]::Min(220, $_.Line.Length)) }
if (Test-Path "$bin\$Label.callers.log") { "callers.log lines: " + (Get-Content "$bin\$Label.callers.log").Count } else { "no callers.log" }
