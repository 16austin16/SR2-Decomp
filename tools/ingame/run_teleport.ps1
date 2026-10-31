# Deploy, boot, Continue, then wait for the teleport test (TELEPORT_TEST in hooks.rs must be
# on) to finish its 8 jumps; screenshots each jump. Usage: run_teleport.ps1 <label>
# (logs to bin/<label>.hook.log and bin/<label>.events.log).
param([string]$Label = "teleport")
$g = "E:\SteamGames\steamapps\common\Saints Row 2"
$bin = "F:\Saints Row 2 Decomp\bin"
$input = "F:\Saints Row 2 Decomp\tools\ingame\sr2input.ps1"
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) { "game already running"; exit 1 }
Copy-Item "F:\Saints Row 2 Decomp\target\i686-pc-windows-msvc\release\dinput8.dll" "$g\dinput8.dll" -Force
Remove-Item "$g\sr2-hook.log", "$g\sr2-streamman.log", "$g\sr2-streamman-seq.log" -ErrorAction SilentlyContinue
Start-Process "steam://rungameid/9480"
$log = "$g\sr2-hook.log"
$deadline = (Get-Date).AddSeconds(150)
while ((Get-Date) -lt $deadline -and -not ((Test-Path $log) -and (Select-String -Path $log -Pattern "find_entry verify" -Quiet))) {
    if ((Test-Path $log) -and -not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED during boot"; break }
    Start-Sleep 2
}
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) {
    foreach ($i in 1..5) { & $input key "1C" | Out-Null; & $input key "01" | Out-Null }
    $deadline = (Get-Date).AddSeconds(600)
    $shot = 0
    while ((Get-Date) -lt $deadline -and -not (Select-String -Path $log -Pattern "teleport test: done" -Quiet)) {
        if (-not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED during test"; break }
        $jumps = (Select-String -Path $log -Pattern "teleport test: frame .* jump").Count
        if ($jumps -gt $shot) { Start-Sleep 8; & $input shot "$bin\$Label.jump$jumps.png" | Out-Null; $shot = $jumps }
        Start-Sleep 2
    }
    # Let the last jump's streaming settle and the next 3600-frame summary write the events.
    $n = (Select-String -Path $log -Pattern "game_frame_tick: frame").Count
    $deadline = (Get-Date).AddSeconds(150)
    while ((Get-Date) -lt $deadline -and (Select-String -Path $log -Pattern "game_frame_tick: frame").Count -le $n) {
        if (-not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { break }
        Start-Sleep 3
    }
    Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
    Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
}
Copy-Item $log "$bin\$Label.hook.log" -ErrorAction SilentlyContinue
Copy-Item "$g\sr2-streamman.log" "$bin\$Label.events.log" -ErrorAction SilentlyContinue
Copy-Item "$g\sr2-streamman-seq.log" "$bin\$Label.seq.log" -ErrorAction SilentlyContinue
Get-Content $log | Select-String -Pattern "teleport test|streamman live|EXITED|panic" | ForEach-Object { $_.Line.Substring(0, [Math]::Min(330, $_.Line.Length)) }
