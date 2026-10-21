# Real keystrokes into the WSLg-hosted Linux cmux window (Windows SendInput -> msrdc -> Wayland/X),
# verified through the cmux socket CLI inside WSL.
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class K2 {
  [StructLayout(LayoutKind.Sequential)] struct KI { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential, Size = 40)] struct IN { public uint type; public KI ki; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, IN[] i, int size);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);
  static void K(ushort vk, bool up) { var i = new IN[1]; i[0].type = 1; i[0].ki.vk = vk; i[0].ki.scan = (ushort)MapVirtualKey(vk, 0);
    i[0].ki.flags = (up ? 2u : 0u) | ((vk >= 0x21 && vk <= 0x28) ? 1u : 0u); SendInput(1, i, Marshal.SizeOf<IN>()); }
  public static void Chord(params ushort[] keys) { foreach (var k in keys) K(k, false); System.Threading.Thread.Sleep(40); for (int j = keys.Length - 1; j >= 0; j--) K(keys[j], true); }
}
"@
$CTRL=0x11; $SHIFT=0x10; $ALT=0x12; $TAB=0x09; $PLUS=0xBB; $MINUS=0xBD; $T=0x54; $W=0x57; $N=0x4E; $K1=0x31; $D=0x44
$hwnd = [IntPtr][long]$args[0]
function Invoke-Cmux([string]$a) { (wsl.exe -d Ubuntu -- /home/trayn/src/cmux-linux/target/release/cmux --json $a.Split(' ')) -join "`n" }
function Tabs { Invoke-Cmux "list-tabs" | ConvertFrom-Json }
function NTabs { @((Tabs).tabs).Count }
function SelTab { $t = @((Tabs).tabs); for ($i=0; $i -lt $t.Count; $i++) { if ($t[$i].selected) { return $i } } }
function NPanes { (@((Tabs).tabs) | Where-Object selected).panes }
function NWs { @((Invoke-Cmux "list-workspaces" | ConvertFrom-Json).workspaces).Count }
function Focus { for ($i=0; $i -lt 10; $i++) { if ([K2]::GetForegroundWindow() -eq $hwnd) { return }; [K2]::Chord($ALT); [K2]::SetForegroundWindow($hwnd) | Out-Null; Start-Sleep -Milliseconds 300 }; throw "could not focus the cmux window" }
function Press([ushort[]]$keys) { Focus; Start-Sleep -Milliseconds 250; [K2]::Chord($keys); Start-Sleep -Milliseconds 1300 }
function Ok($name, $got, $want) { if ("$got" -eq "$want") { "PASS  $name ($got)" } else { "FAIL  $name (got $got, want $want)" } }

$t0 = NTabs;  Press $CTRL,$SHIFT,$T;    Ok "Ctrl+Shift+T opens a tab" (NTabs) ($t0+1)
$p0 = NPanes; Press $ALT,$SHIFT,$PLUS;  Ok "Alt+Shift+= splits" (NPanes) ($p0+1)
$p1 = NPanes; Press $ALT,$SHIFT,$MINUS; Ok "Alt+Shift+- splits" (NPanes) ($p1+1)
$p2 = NPanes; Press $CTRL,$SHIFT,$W;    Ok "Ctrl+Shift+W closes a pane" (NPanes) ($p2-1)
Press $CTRL,$K1;                         Ok "Ctrl+1 selects the first tab" (SelTab) 0
Press $CTRL,$TAB;                        Ok "Ctrl+Tab selects the next tab" (SelTab) 1
$w0 = NWs;    Press $CTRL,$SHIFT,$N;     Ok "Ctrl+Shift+N opens a workspace" (NWs) ($w0+1)
$t1 = NTabs; $p3 = NPanes; Press $CTRL,$W; Ok "Ctrl+W goes to the shell (nothing closes)" "$(NTabs)/$(NPanes)" "$t1/$p3"

