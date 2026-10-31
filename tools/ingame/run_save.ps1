# Two-phase save test.
#   run_save.ps1 open <label>  : deploy, boot, Continue into the save, open Save Game list, screenshot
#   run_save.ps1 save <label>  : press Enter on the highlighted entry (must be NEW SAVE GAME), OK, close, copy logs
param([string]$Phase, [string]$Label = "save")
$g = "E:\SteamGames\steamapps\common\Saints Row 2"
$bin = "F:\Saints Row 2 Decomp\bin"
$in = "$PSScriptRoot\sr2input.ps1"
$log = "$g\sr2-hook.log"
if ($Phase -eq "open") {
    if (Get-Process SR2_pc -ErrorAction SilentlyContinue) { "game already running"; exit 1 }
    Copy-Item "F:\Saints Row 2 Decomp\target\i686-pc-windows-msvc\release\dinput8.dll" "$g\dinput8.dll" -Force
    Remove-Item "$g\sr2-hook.log", "$g\sr2-fileopen.log", "$g\sr2-fileread.log", "$g\sr2-filewrite.log", "$g\sr2-fileclose.log" -ErrorAction SilentlyContinue
    Start-Process "steam://rungameid/9480"
    $deadline = (Get-Date).AddSeconds(150)
    while ((Get-Date) -lt $deadline -and -not ((Test-Path $log) -and (Select-String -Path $log -Pattern "find_entry verify" -Quiet))) { Start-Sleep 2 }
    foreach ($i in 1..5) { & $in key "1C" | Out-Null; & $in key "01" | Out-Null }
    $deadline = (Get-Date).AddSeconds(120)
    while ((Get-Date) -lt $deadline -and -not (Select-String -Path $log -Pattern 'unmount_temp_all: before temp 1 \["city_load' -Quiet)) {
        if (-not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED during load"; exit 1 }
        Start-Sleep 2
    }
    Start-Sleep 10
    & $in key "01" | Out-Null; Start-Sleep -Milliseconds 1500
    & $in key "2A" | Out-Null; Start-Sleep -Milliseconds 800
    & $in key "1C" | Out-Null; Start-Sleep -Milliseconds 1500
    & $in shot "$bin\shot.png"
}
elseif ($Phase -eq "save") {
    & $in key "1C" | Out-Null; Start-Sleep -Seconds 5
    & $in shot "$bin\shot.png" | Out-Null
    & $in key "1C" | Out-Null; Start-Sleep -Seconds 4   # OK, then let the call logs flush
    Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
    Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
    foreach ($n in "hook", "filewrite", "fileclose") {
        $src = if ($n -eq "hook") { $log } else { "$g\sr2-$n.log" }
        Copy-Item $src "$bin\$Label.$n.log" -ErrorAction SilentlyContinue
    }
    Get-Content $log | Select-String 'file_(write|close|open \(write\))|mismatch|EXITED' | ForEach-Object { $_.Line.Substring(0, [Math]::Min(230, $_.Line.Length)) }
    foreach ($n in "filewrite", "fileclose") { if (Test-Path "$bin\$Label.$n.log") { "$n lines: " + (Get-Content "$bin\$Label.$n.log").Count } }
    Get-ChildItem "$env:LOCALAPPDATA\THQ\Saints Row 2" -Filter *.bbsave | Select-Object Name, Length, LastWriteTime | Format-Table -AutoSize | Out-String
}
