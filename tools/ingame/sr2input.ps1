param(
    [string]$Action,          # shot | key | hold | move | wheel | click | rclick | focus
    [string]$Arg = "",        # key: scancode hex list (e.g. "1C" or "11,11"); click: "x,y" window-relative; shot: out path
    [int]$HoldMs = 80
)
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Runtime.InteropServices;
public static class SR2In {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] public struct INPUT {
        [FieldOffset(0)] public uint type;
        [FieldOffset(8)] public MOUSEINPUT mi;
        [FieldOffset(8)] public KEYBDINPUT ki;
    }
    [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref System.Drawing.Point p);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    public static void Key(ushort scan, bool up, bool ext) {
        var i = new INPUT[1];
        i[0].type = 1;
        i[0].ki.scan = scan;
        i[0].ki.flags = 0x8u | (up ? 0x2u : 0u) | (ext ? 0x1u : 0u); // SCANCODE, KEYUP, EXTENDEDKEY
        SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void MoveRel(int dx, int dy) {
        var i = new INPUT[1];
        i[0].type = 0;
        i[0].mi.dx = dx; i[0].mi.dy = dy; i[0].mi.flags = 0x1u; // MOVE (relative)
        SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Wheel(int delta) {
        var i = new INPUT[1];
        i[0].type = 0;
        i[0].mi.data = (uint)delta; i[0].mi.flags = 0x800u; // WHEEL
        SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Mouse(uint flags) {
        var i = new INPUT[1];
        i[0].type = 0;
        i[0].mi.flags = flags;
        SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
    }
}
"@
$p = Get-Process SR2_pc -ErrorAction Stop | Select-Object -First 1
$h = $p.MainWindowHandle
$origin = New-Object System.Drawing.Point 0, 0
[void][SR2In]::ClientToScreen($h, [ref]$origin)
$cr = New-Object SR2In+RECT
[void][SR2In]::GetClientRect($h, [ref]$cr)

switch ($Action) {
    "focus" { [void][SR2In]::SetForegroundWindow($h); "focused client $($cr.R)x$($cr.B) at $($origin.X),$($origin.Y)" }
    "key" {
        [void][SR2In]::SetForegroundWindow($h); Start-Sleep -Milliseconds 150
        foreach ($s in $Arg.Split(",")) {
            $ext = $s.StartsWith("E0"); $sc = [Convert]::ToUInt16($(if ($ext) { $s.Substring(2) } else { $s }), 16)
            [SR2In]::Key($sc, $false, $ext); Start-Sleep -Milliseconds $HoldMs
            [SR2In]::Key($sc, $true, $ext); Start-Sleep -Milliseconds 250
        }
        "sent $Arg"
    }
    "click" {
        [void][SR2In]::SetForegroundWindow($h); Start-Sleep -Milliseconds 150
        $xy = $Arg.Split(","); [void][SR2In]::SetCursorPos($origin.X + [int]$xy[0], $origin.Y + [int]$xy[1])
        Start-Sleep -Milliseconds 150
        [SR2In]::Mouse(0x2); Start-Sleep -Milliseconds $HoldMs; [SR2In]::Mouse(0x4)
        "clicked $Arg"
    }
    "hold" {
        # Arg "scan[,scan...]": press them together for HoldMs, then release.
        [void][SR2In]::SetForegroundWindow($h); Start-Sleep -Milliseconds 150
        $keys = @(); foreach ($s in $Arg.Split(",")) { $e = $s.StartsWith("E0"); $keys += ,@([uint16][Convert]::ToUInt16($(if ($e) { $s.Substring(2) } else { $s }), 16), [bool]$e) }
        foreach ($k in $keys) { [SR2In]::Key([uint16]$k[0], $false, [bool]$k[1]); Start-Sleep -Milliseconds 30 }
        Start-Sleep -Milliseconds $HoldMs
        foreach ($k in $keys) { [SR2In]::Key([uint16]$k[0], $true, [bool]$k[1]) }
        "held $Arg for $HoldMs ms"
    }
    "move" {
        # Arg "dx,dy[,steps]": relative mouse motion split over steps (16 ms apart).
        [void][SR2In]::SetForegroundWindow($h); Start-Sleep -Milliseconds 100
        $v = $Arg.Split(","); $n = if ($v.Count -gt 2) { [int]$v[2] } else { 20 }
        foreach ($i in 1..$n) { [SR2In]::MoveRel([int]([int]$v[0] / $n), [int]([int]$v[1] / $n)); Start-Sleep -Milliseconds 16 }
        "moved $Arg"
    }
    "wheel" {
        [void][SR2In]::SetForegroundWindow($h); Start-Sleep -Milliseconds 100
        [SR2In]::Wheel([int]$Arg); "wheel $Arg"
    }
    "rclick" {
        [void][SR2In]::SetForegroundWindow($h); Start-Sleep -Milliseconds 100
        [SR2In]::Mouse(0x8); Start-Sleep -Milliseconds $HoldMs; [SR2In]::Mouse(0x10); "right-clicked"
    }
    "shot" {
        $w = $cr.R; $hh = $cr.B
        $bmp = New-Object System.Drawing.Bitmap $w, $hh
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($origin.X, $origin.Y, 0, 0, (New-Object System.Drawing.Size $w, $hh))
        $small = New-Object System.Drawing.Bitmap $bmp, ([int]($w / 2)), ([int]($hh / 2))
        $small.Save($Arg, [System.Drawing.Imaging.ImageFormat]::Png)
        "saved ${w}x${hh} (half-size) to $Arg"
    }
}

