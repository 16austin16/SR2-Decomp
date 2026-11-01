# Deploy, boot, Continue into the save, then drive the F7 menu. Default: Player
# options (god mode, cash, respect), then Mission logging (on, two F8 marks, off).
# -Options: infinite ammo, clear wanted level, time +2 h / -1 h instead.
# Screenshots to bin/menu_*.png.
# -World: weather (two picks, then normal), the game cheats list (first cheat on, off),
# then infinite ammo. -Cheats: just the cheats list.
param([switch]$Options, [switch]$World, [switch]$Cheats)
$g = "E:\SteamGames\steamapps\common\Saints Row 2"
$bin = "F:\Saints Row 2 Decomp\bin"
$in = "$PSScriptRoot\sr2input.ps1"
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) { "game already running"; exit 1 }
Copy-Item "F:\Saints Row 2 Decomp\target\i686-pc-windows-msvc\release\dinput8.dll" "$g\dinput8.dll" -Force
Remove-Item "$g\sr2-hook.log", "$g\sr2-mission.log" -ErrorAction SilentlyContinue
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
function k($s) { & $in key $s | Out-Null; Start-Sleep -Milliseconds 400 }
function shot($n) { & $in shot "$bin\menu_$n.png" | Out-Null }
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) {
    if ($Cheats) {
        k "41"; k "E050"; k "1C"; k "E050"; k "1C"; Start-Sleep 1; shot "c1_cheats"
        foreach ($i in 1..10) { k "E050" }; shot "c2_cheats_more"
        k "41"; Start-Sleep 2
        Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
        Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
    } elseif ($World) {
        k "41"; k "E050"; k "1C"; shot "w1_world"
        k "E04D"; k "1C"; Start-Sleep 3; shot "w2_weather1"
        k "E04D"; k "E04D"; k "1C"; Start-Sleep 3; shot "w3_weather3"
        k "E04B"; k "E04B"; k "E04B"; k "1C"; Start-Sleep 2
        k "E050"; k "1C"; Start-Sleep 1; shot "w4_cheats"
        k "1C"; Start-Sleep 2; shot "w5_cheat_on"; k "1C"; Start-Sleep 2; shot "w6_cheat_off"
        k "0E"; k "1C"; k "E050"; k "1C"; shot "w7_ammo"
        k "41"; Start-Sleep 2
        Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
        Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
    } elseif ($Options) {
        k "41"; k "1C"; k "E050"; k "1C"; k "E050"; k "E050"; k "E050"; k "E050"; k "1C"; shot "6_ammo_wanted"
        k "E050"; k "E050"; shot "7_time_before"; k "E04D"; Start-Sleep 1; k "E04D"; Start-Sleep 1; shot "8_time_plus2"; k "E04B"; Start-Sleep 1; shot "9_time_minus1"
        k "41"; Start-Sleep 2
        Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
        Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
    } else {
    k "41"; shot "1_main"
    k "1C"; shot "2_player"
    k "1C"; k "E050"; k "E050"; k "1C"; k "E050"; k "1C"; shot "3_god_cash_respect"
    k "0E"; k "E050"; k "1C"; k "1C"; Start-Sleep 5; k "42"; Start-Sleep 5; k "42"; Start-Sleep 3; shot "4_logging"
    k "1C"; k "41"; Start-Sleep 2; shot "5_closed"
    Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
    Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
    }
}
Copy-Item $log "$bin\menu.hook.log" -ErrorAction SilentlyContinue
Copy-Item "$g\sr2-mission.log" "$bin\menu.mission.log" -ErrorAction SilentlyContinue
Get-Content $log | Select-String "menu:|missionlog|memory:|CRASH|panic"
if (Test-Path "$bin\menu.mission.log") { Get-Content "$bin\menu.mission.log" | Select-String "^==" ; "mission log lines: " + (Get-Content "$bin\menu.mission.log").Count }
