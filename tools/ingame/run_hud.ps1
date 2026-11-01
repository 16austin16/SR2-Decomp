# Deploy, boot, Continue into the save, then exercise the vint HUD for the Lua
# binding port (vint_lua.rs): the HUD, the pause menu (Esc) and its map tab, walking.
# Screenshots to bin/hud_*.png; prints the vint_lua summary lines.
$g = "E:\SteamGames\steamapps\common\Saints Row 2"
$bin = "F:\Saints Row 2 Decomp\bin"
$in = "$PSScriptRoot\sr2input.ps1"
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) { "game already running"; exit 1 }
Copy-Item "F:\Saints Row 2 Decomp\target\i686-pc-windows-msvc\release\dinput8.dll" "$g\dinput8.dll" -Force
Remove-Item "$g\sr2-hook.log" -ErrorAction SilentlyContinue
Start-Process "steam://rungameid/9480"
$log = "$g\sr2-hook.log"
$deadline = (Get-Date).AddSeconds(150)
while ((Get-Date) -lt $deadline -and -not ((Test-Path $log) -and (Select-String -Path $log -Pattern "find_entry verify" -Quiet))) { Start-Sleep 2 }
foreach ($i in 1..5) { & $in key "1C" | Out-Null; & $in key "01" | Out-Null }
$deadline = (Get-Date).AddSeconds(240)
while ((Get-Date) -lt $deadline -and -not (Select-String -Path $log -Pattern "frame 3600" -Quiet)) {
    if (-not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED during load"; break }
    Start-Sleep 3
}
function k($s) { & $in key $s | Out-Null; Start-Sleep -Milliseconds 600 }
function shot($n) { & $in shot "$bin\hud_$n.png" | Out-Null }
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) {
    Start-Sleep 3; shot "1_hud"
    k "01"; Start-Sleep 2; shot "2_pause"
    k "E04D"; Start-Sleep 2; shot "3_pause_tab"
    k "E04D"; Start-Sleep 2; shot "4_pause_tab2"
    k "01"; Start-Sleep 1; k "01"; Start-Sleep 2
    k "0F"; Start-Sleep 3; shot "5_map"; k "0F"; Start-Sleep 2
    & $in -Action hold -Arg "11" -HoldMs 4000 | Out-Null; Start-Sleep 2; shot "6_walked"
    Start-Sleep 20
    Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
    Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
}
Copy-Item $log "$bin\hud.hook.log" -ErrorAction SilentlyContinue
Get-Content $log | Select-String "vint_lua|CRASH|panic"
