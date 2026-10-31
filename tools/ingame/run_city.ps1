# Deploy the hook DLL, boot SR2, Continue into the save, wait, close. Prints the
# non-mount log lines. Usage: run_city.ps1 <label>   (copies logs to bin/<label>.*)
param([string]$Label = "run")
$g = "E:\SteamGames\steamapps\common\Saints Row 2"
$bin = "F:\Saints Row 2 Decomp\bin"
$input = "$PSScriptRoot\sr2input.ps1"
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) { "game already running"; exit 1 }
Copy-Item "F:\Saints Row 2 Decomp\target\i686-pc-windows-msvc\release\dinput8.dll" "$g\dinput8.dll" -Force
Remove-Item "$g\sr2-hook.log", "$g\sr2-fileopen.log", "$g\sr2-fileread.log" -ErrorAction SilentlyContinue
Start-Process "steam://rungameid/9480"
$log = "$g\sr2-hook.log"
$deadline = (Get-Date).AddSeconds(150)
while ((Get-Date) -lt $deadline -and -not ((Test-Path $log) -and (Select-String -Path $log -Pattern "find_entry verify" -Quiet))) {
    if ((Test-Path $log) -and -not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED during boot"; break }
    Start-Sleep 2
}
if (Get-Process SR2_pc -ErrorAction SilentlyContinue) {
    foreach ($i in 1..5) { & $input key "1C" | Out-Null; & $input key "01" | Out-Null }
    # Wait for the frame-3600 summary (covers the whole save load).
    $deadline = (Get-Date).AddSeconds(240)
    while ((Get-Date) -lt $deadline -and -not (Select-String -Path $log -Pattern "frame 3600" -Quiet)) {
        if (-not (Get-Process SR2_pc -ErrorAction SilentlyContinue)) { "GAME EXITED during load"; break }
        Start-Sleep 3
    }
    & $input shot "$bin\shot.png" | Out-Null
    Stop-Process -Name SR2_pc -ErrorAction SilentlyContinue
    Wait-Process -Name SR2_pc -Timeout 20 -ErrorAction SilentlyContinue
}
Copy-Item $log "$bin\$Label.hook.log" -ErrorAction SilentlyContinue
Copy-Item "$g\sr2-fileopen.log" "$bin\$Label.fileopen.log" -ErrorAction SilentlyContinue; Copy-Item "$g\sr2-fileread.log" "$bin\$Label.fileread.log" -ErrorAction SilentlyContinue
Get-Content $log | Select-String -NotMatch '^mount "' | ForEach-Object { $_.Line.Substring(0, [Math]::Min(220, $_.Line.Length)) }
if (Test-Path "$bin\$Label.fileopen.log") { "file_open lines: " + (Get-Content "$bin\$Label.fileopen.log").Count }

if (Test-Path "$bin\$Label.fileread.log") { "file_read lines: " + (Get-Content "$bin\$Label.fileread.log").Count }

