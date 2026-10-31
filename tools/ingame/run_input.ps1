# Deploy, boot, Continue into the save, then play a scripted stretch of keyboard and
# mouse input (walk, sprint, jump, look around, wheel, clicks, pause menu) and close
# after the next 3600-frame summary. Usage: run_input.ps1 <label>
param([string]$Label = "input")
$g = "E:\SteamGames\steamapps\common\Saints Row 2"
$bin = "F:\Saints Row 2 Decomp\bin"
$in = "$PSScriptRoot\sr2input.ps1"
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) { "game already running"; exit 1 }
Copy-Item "F:\Saints Row 2 Decomp\target\i686-pc-windows-msvc\release\dinput8.dll" "$g\dinput8.dll" -Force
Remove-Item "$g\sr2-hook.log" -ErrorAction SilentlyContinue
Start-Process "steam://rungameid/9480"
$log = "$g\sr2-hook.log"
$deadline = (Get-Date).AddSeconds(150)
while ((Get-Date) -lt $deadline -and -not ((Test-Path $log) -and (Select-String -Path $log -Pattern "find_entry verify" -Quiet))) {
    if ((Test-Path $log) -and -not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED during boot"; break }
    Start-Sleep 2
}
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) {
    foreach ($i in 1..5) { & $in key "1C" | Out-Null; & $in key "01" | Out-Null }
    Start-Sleep 40   # the save loads
    & $in shot "$bin\$Label.start.png" | Out-Null
    foreach ($round in 1..2) {
        & $in hold "11" -HoldMs 2500 | Out-Null            # W
        & $in hold "11,2A" -HoldMs 2000 | Out-Null         # W + LShift (sprint)
        & $in move "400,0,30" | Out-Null                   # look right
        & $in hold "1E" -HoldMs 1000 | Out-Null            # A
        & $in move "-250,120,30" | Out-Null                # look left and down
        & $in hold "1F" -HoldMs 800 | Out-Null             # S
        & $in hold "20" -HoldMs 800 | Out-Null             # D
        & $in key "39" | Out-Null                          # Space (jump)
        & $in hold "11" -HoldMs 3000 | Out-Null            # W held long (key repeat)
        & $in wheel 120 | Out-Null; Start-Sleep 1
        & $in wheel -120 | Out-Null; Start-Sleep 1
        & $in rclick -HoldMs 600 | Out-Null
        & $in move "0,-120,10" | Out-Null
        & $in key "01" | Out-Null; Start-Sleep 2           # Esc: pause menu
        & $in key "E050,E050,E048" | Out-Null              # arrows in the menu
        & $in key "01" | Out-Null; Start-Sleep 2           # back to the game
        & $in hold "1D,11" -HoldMs 1000 | Out-Null         # Ctrl + W (modifiers)
        & $in key "23,24" | Out-Null                       # H, J
    }
    & $in shot "$bin\$Label.end.png" | Out-Null
    $n = (Select-String -Path $log -Pattern "game_frame_tick: frame").Count
    $deadline = (Get-Date).AddSeconds(90)
    while ((Get-Date) -lt $deadline -and (Select-String -Path $log -Pattern "game_frame_tick: frame").Count -le $n) {
        if (-not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED"; break }
        Start-Sleep 3
    }
    Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
    Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
}
Copy-Item $log "$bin\$Label.hook.log" -ErrorAction SilentlyContinue
Get-Content $log | Select-String -Pattern "input|EXITED|panic" | ForEach-Object { $_.Line.Substring(0, [Math]::Min(400, $_.Line.Length)) }
