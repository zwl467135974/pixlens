# PixLens Thumbnail Diagnose & Fix (run via PixLensThumbDiag.bat)
# Report: Desktop\PixLens-thumb-diag.txt
param([switch]$Fix)
$ErrorActionPreference = "Continue"
$report = New-Object System.Collections.Generic.List[string]
function Log($s){ $report.Add($s); Write-Host $s }

Log "==== PixLens Thumbnail Diagnostics ===="
Log "Time: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Log "User: $env:USERNAME  Machine: $env:COMPUTERNAME"
Log "OS: $([Environment]::OSVersion.VersionString)"

$clsid = '{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}'
$TestPsdB64 = @'
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
'@
$thumbKey = '{E357FCCD-A995-4576-B01F-234630154E96}'

# 1. install location + files
$inst = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\PixLens" -ErrorAction SilentlyContinue).InstallLocation
if(-not $inst){ $inst = (Get-ItemProperty "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\PixLens" -ErrorAction SilentlyContinue).InstallLocation }
if($inst){ $inst = $inst.Trim('"') }
Log ""
Log "[1] Install location: $inst"
$inproc = (Get-ItemProperty "HKCU:\Software\Classes\CLSID\$clsid\InprocServer32" -ErrorAction SilentlyContinue).'(default)'
Log "[1] CLSID InprocServer32 (HKCU): $inproc"
if($inproc){ Log ("[1] thumb dll exists: " + (Test-Path $inproc)) }
$dir = $null
if($inproc -and (Test-Path $inproc)){ $dir = Split-Path $inproc }
elseif($inst -and (Test-Path (Join-Path $inst 'pixlens_thumb_cpp.dll'))){ $dir = $inst }
if($dir){
  foreach($f in @('pixlens_thumb_cpp.dll','pixlens_psd.dll')){
    $p = Join-Path $dir $f
    if(Test-Path $p){ $i = Get-Item $p; Log ("[1] {0} : {1:N0} bytes  {2}" -f $f, $i.Length, $i.LastWriteTime) }
    else { Log "[1] $f : MISSING  <-- problem" }
  }
  $exeP = if($inst){ Join-Path $inst 'pixlens.exe' } else { Join-Path $dir 'pixlens.exe' }
  if(Test-Path $exeP){ Log ("[1] pixlens.exe (install dir): {0:N0} bytes" -f (Get-Item $exeP).Length) }
  else { Log "[1] pixlens.exe : MISSING in install dir" }
} else { Log "[1] install dir NOT FOUND <-- problem" }

# 2. per-user vs per-machine registration
Log ""
Log "[2] CLSID registered: HKCU=$(Test-Path "HKCU:\Software\Classes\CLSID\$clsid")  HKLM=$(Test-Path "HKLM:\Software\Classes\CLSID\$clsid")"

# 3. association chain (ProgID precedence!)
Log ""
Log "[3] Association chain:"
foreach($ext in @('.psd','.psb')){
  $hkcuExt = (Get-ItemProperty "HKCU:\Software\Classes\$ext" -ErrorAction SilentlyContinue).'(default)'
  $hkcrExt = (Get-ItemProperty "Registry::HKEY_CLASSES_ROOT\$ext" -ErrorAction SilentlyContinue).'(default)'
  $uc = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$ext\UserChoice" -ErrorAction SilentlyContinue).ProgId
  Log "  $ext : HKCU-default=$hkcuExt  HKCR-default=$hkcrExt  UserChoice=$uc"
  $prog = if($uc){$uc}elseif($hkcuExt){$hkcuExt}elseif($hkcrExt){$hkcrExt}else{$null}
  if($prog){
    $pSh = (Get-ItemProperty "Registry::HKEY_CLASSES_ROOT\$prog\shellex\$thumbKey" -ErrorAction SilentlyContinue).'(default)'
    $note = ''
    if($pSh -and $pSh -ne $clsid){ $note = '  <-- OTHER HANDLER OVERRIDES OURS' }
    Log "  winning ProgID = $prog ; its shellex = $pSh$note"
  }
  $eSh = (Get-ItemProperty "HKCU:\Software\Classes\$ext\shellex\$thumbKey" -ErrorAction SilentlyContinue).'(default)'
  Log "  extension-level shellex (ours) = $eSh"
}

# 4. explorer settings
Log ""
Log "[4] Explorer settings:"
$iconsOnly = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced" -ErrorAction SilentlyContinue).IconsOnly
$note4 = ''
if($iconsOnly -eq 1){ $note4 = '  <-- PROBLEM: folder option hides ALL thumbnails' }
Log "  IconsOnly (1=never show thumbnails): $iconsOnly$note4"
$policy = (Get-ItemProperty "HKCU:\Software\Policies\Microsoft\Windows\Explorer" -ErrorAction SilentlyContinue).DisableThumbnails
Log "  Policy DisableThumbnails: $policy"

# 5. security software
Log ""
Log "[5] Security software running:"
$known = @('360tray','360safe','ZhuDongFangYu','HipsTray','HipsDaemon','wsctrl','QQPCRTP','QMDL','kwsprotect','usysdiag')
$found = Get-Process -ErrorAction SilentlyContinue | Where-Object { $known -contains $_.Name } | Select-Object -ExpandProperty Name -Unique
if($found){ Log ("  detected: " + ($found -join ', ') + '  <-- may block DllHost loading unsigned DLL') } else { Log "  none of the known list detected" }

# 6. functional shell test
Log ""
Log "[6] Functional shell thumbnail test:"
$testDir = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'PixLensDiag'
New-Item -ItemType Directory -Path $testDir -Force | Out-Null
$testPsd = Join-Path $testDir 'pixlens_test.psd'
[IO.File]::WriteAllBytes($testPsd, [Convert]::FromBase64String($TestPsdB64))
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class PXD {
  [StructLayout(LayoutKind.Sequential)]
  public struct SIZE { public int cx; public int cy; }
  [ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  public interface IShellItemImageFactory {
    [PreserveSig] int GetImage(SIZE size, int flags, out IntPtr phbm);
  }
  [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
  static extern void SHCreateItemFromParsingName(string path, IntPtr pbc, ref Guid riid, out IShellItemImageFactory ppv);
  public static int Extract(string path, int size, string outPng) {
    Guid iid = typeof(IShellItemImageFactory).GUID;
    IShellItemImageFactory f;
    SHCreateItemFromParsingName(path, IntPtr.Zero, ref iid, out f);
    SIZE s; s.cx = size; s.cy = size;
    IntPtr hb;
    int hr = f.GetImage(s, 0x8, out hb);
    if (hr != 0) return hr;
    using (var img = System.Drawing.Image.FromHbitmap(hb)) { img.Save(outPng, System.Drawing.Imaging.ImageFormat.Png); }
    return 0;
  }
}
"@
$png = Join-Path $testDir 'result.png'
if(Test-Path $png){ Remove-Item $png -Force }
$hr = -1
try { $hr = [PXD]::Extract($testPsd, 256, $png) } catch { Log ("  EX: " + $_.Exception.Message) }
$verdict = 'see hr'
if($hr -eq 0 -and (Test-Path $png)){ $verdict = 'SUCCESS' }
elseif($hr -eq 0x8004B200){ $verdict = 'handler-invoked but FAILED (registration ok, decode/blocked)' }
elseif($hr -eq 0x80070057){ $verdict = 'E_INVALIDARG (shell refused)' }
elseif($hr -eq 0x80004005){ $verdict = 'E_FAIL' }
elseif($hr -eq 0x80040154){ $verdict = 'REGDB_E_CLASSNOTREG (CLSID lookup/load FAILED - policy or bitness)' }
Log ("  GetImage hr = 0x{0:X8}  {1}" -f $hr, $verdict)

# 6b. deep probe: bitness, LoadLibrary, direct COM activation, policy
Log ""
Log "[6b] Deep probe:"
$b64 = [Environment]::Is64BitProcess; $os64 = [Environment]::Is64BitOS
$bnote = if(-not $b64){' <-- 32bit PS: registry view differs, rerun via System32 powershell'}else{''}
Log ("  PS process: 64bit={0}  OS 64bit={1}{2}" -f $b64, $os64, $bnote)
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class PXL {
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
  public static extern IntPtr LoadLibraryW(string f);
  [DllImport("kernel32.dll")] public static extern bool FreeLibrary(IntPtr h);
  [ComImport, Guid("e357fccd-a995-4576-b01f-234630154e96"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  public interface IThumbProv {
    [PreserveSig] int GetThumbnail(uint cx, out IntPtr hbmp, out int alpha);
  }
  [DllImport("ole32.dll")]
  public static extern int CoCreateInstance(ref Guid clsid, IntPtr pUnkOuter, uint dwClsContext, ref Guid riid, out IThumbProv ppv);
}
"@
if($inproc -and (Test-Path $inproc)){
  $h = [PXL]::LoadLibraryW($inproc)
  if($h -ne [IntPtr]::Zero){
    Log "  LoadLibrary(thumb dll) OK"
    [void][PXL]::FreeLibrary($h)
  } else {
    $e = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    $why = switch($e){ 126 {'126 module not found (missing dependency)'} 127 {'127 proc not found'} 5 {'5 access denied (policy/AV?)'} 193 {'193 bad format (bitness?)'} 998 {'998 access violation in init'} default {"$e"} }
    Log "  LoadLibrary(thumb dll) FAILED: $why  <-- KEY FINDING"
  }
  $c = New-Object Guid('2B2E7C27-BC52-4521-9A56-87BC2DFC7639')
  $r = New-Object Guid('E357FCCD-A995-4576-B01F-234630154E96')
  $prov = $null
  $hrC = [PXL]::CoCreateInstance([ref]$c, [IntPtr]::Zero, 1, [ref]$r, [ref]$prov)
  if($hrC -eq 0){
    Log "  CoCreateInstance(thumbnail provider) OK - COM registration is WORKING in this process"
  } else {
    Log ("  CoCreateInstance FAILED hr=0x{0:X8}  <-- COM cannot activate class here" -f $hrC)
  }
} else { Log "  skipped (dll path unknown)" }
$appLocker = Test-Path "HKLM:\SOFTWARE\Policies\Microsoft\Windows\SrpV2"
$srp = Test-Path "HKLM:\Software\Policies\Microsoft\Windows\Safer\CodeIdentifiers"
Log ("  AppLocker policy: {0}  SRP policy: {1} $(if($appLocker -or $srp){'<-- DLL RULES MAY BLOCK LOADING'})" -f $appLocker, $srp)
foreach($rt in @('vcruntime140.dll','msvcp140.dll')){
  Log ("  $rt in System32: " + (Test-Path "$env:SystemRoot\System32\$rt"))
}

# 6c. whose account is the desktop Explorer running under?
Log ""
Log "[6c] Explorer process owner(s):"
try {
  Get-CimInstance Win32_Process -Filter "Name='explorer.exe'" -ErrorAction Stop | ForEach-Object {
    $o = Invoke-CimMethod -InputObject $_ -MethodName GetOwner
    Log ("  PID {0} -> {1}\{2} $(if("$($o.Domain)\$($o.User)" -ne "$env:USERDOMAIN\$env:USERNAME"){'<-- DIFFERENT from current user!'})" -f $_.ProcessId, $o.Domain, $o.User)
  }
} catch { Log "  probe failed: $($_.Exception.Message)" }

# 7. fix
if($Fix){
  Log ""
  Log "[7] FIX applied:"
  if($dir -and (Test-Path (Join-Path $dir 'pixlens_thumb_cpp.dll'))){
    New-Item -Path "HKCU:\Software\Classes\CLSID\$clsid\InprocServer32" -Force | Out-Null
    Set-ItemProperty "HKCU:\Software\Classes\CLSID\$clsid\InprocServer32" '(default)' (Join-Path $dir 'pixlens_thumb_cpp.dll')
    New-ItemProperty -Path "HKCU:\Software\Classes\CLSID\$clsid\InprocServer32" -Name 'ThreadingModel' -Value 'Apartment' -PropertyType String -Force | Out-Null
    Log "  CLSID re-registered -> $dir"
    foreach($ext in @('.psd','.psb')){
      New-Item -Path "HKCU:\Software\Classes\$ext\shellex\$thumbKey" -Force | Out-Null
      Set-ItemProperty "HKCU:\Software\Classes\$ext\shellex\$thumbKey" '(default)' $clsid
      $hkcuExt = (Get-ItemProperty "HKCU:\Software\Classes\$ext" -ErrorAction SilentlyContinue).'(default)'
      $uc = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$ext\UserChoice" -ErrorAction SilentlyContinue).ProgId
      $hkcrExt = (Get-ItemProperty "Registry::HKEY_CLASSES_ROOT\$ext" -ErrorAction SilentlyContinue).'(default)'
      foreach($prog in @($uc,$hkcuExt,$hkcrExt) | Where-Object { $_ } | Select-Object -Unique){
        New-Item -Path "HKCU:\Software\Classes\$prog\shellex\$thumbKey" -Force | Out-Null
        Set-ItemProperty "HKCU:\Software\Classes\$prog\shellex\$thumbKey" '(default)' $clsid
        Log "  shellex registered under ProgID: $prog"
      }
      Log "  shellex registered under extension: $ext"
    }
    Get-Process dllhost -ErrorAction SilentlyContinue | ForEach-Object { try { $_ | Stop-Process -Force -ErrorAction Stop } catch {} }
    Log "  dllhost flushed (restarts on demand)"

    # machine-wide HKLM registration: shell thumbnail extraction may run in a
    # context where HKCU is NOT visible (SYSTEM surrogate / other account's
    # Explorer) -- HKLM is what commercial shell extensions (Adobe etc.) use
    $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if($isAdmin){
      try {
        $dllFull = (Get-ItemProperty "HKCU:\Software\Classes\CLSID\$clsid\InprocServer32" -ErrorAction SilentlyContinue).'(default)'
        if(-not $dllFull){ $dllFull = Join-Path $dir 'pixlens_thumb_cpp.dll' }
        New-Item -Path "HKLM:\Software\Classes\CLSID\$clsid\InprocServer32" -Force | Out-Null
        Set-ItemProperty "HKLM:\Software\Classes\CLSID\$clsid\InprocServer32" '(default)' $dllFull
        New-ItemProperty -Path "HKLM:\Software\Classes\CLSID\$clsid\InprocServer32" -Name 'ThreadingModel' -Value 'Apartment' -PropertyType String -Force | Out-Null
        foreach($ext in @('.psd','.psb')){
          New-Item -Path "HKLM:\Software\Classes\$ext\shellex\$thumbKey" -Force | Out-Null
          Set-ItemProperty "HKLM:\Software\Classes\$ext\shellex\$thumbKey" '(default)' $clsid
        }
        Log "  HKLM machine-wide registration written -> $dllFull (visible to ALL contexts/users)"
      } catch { Log "  HKLM write FAILED: $($_.Exception.Message)" }
    } else { Log "  HKLM skipped: process not elevated -- RIGHT-CLICK bat, run as administrator" }
    if(Test-Path $png){ Remove-Item $png -Force }
    $hr2 = -1
    try { $hr2 = [PXD]::Extract($testPsd, 256, $png) } catch {}
    $v2 = if($hr2 -eq 0 -and (Test-Path $png)){'SUCCESS after fix'}else{'still failing - SEND REPORT BACK'}
    Log ("  retest hr = 0x{0:X8}  {1}" -f $hr2, $v2)

    # fallback: relocate DLLs to an ASCII-safe path under LOCALAPPDATA and re-register
    if(-not ($hr2 -eq 0 -and (Test-Path $png))){
      Log "  [fallback] relocating shell DLLs to ASCII path..."
      $safeDir = Join-Path $env:LOCALAPPDATA 'PixLensShell'
      New-Item -ItemType Directory -Path $safeDir -Force | Out-Null
      $moved = $true
      foreach($f in @('pixlens_thumb_cpp.dll','pixlens_psd.dll')){
        $srcF = Join-Path $dir $f
        $dstF = Join-Path $safeDir $f
        if($srcF -ne $dstF){
          if(Test-Path $srcF){ Copy-Item $srcF $dstF -Force } else { $moved = $false }
        }
      }
      if($moved){
        Set-ItemProperty "HKCU:\Software\Classes\CLSID\$clsid\InprocServer32" '(default)' (Join-Path $safeDir 'pixlens_thumb_cpp.dll')
        Log "  CLSID repointed -> $safeDir"
        Get-Process dllhost -ErrorAction SilentlyContinue | ForEach-Object { try { $_ | Stop-Process -Force -ErrorAction Stop } catch {} }
        if(Test-Path $png){ Remove-Item $png -Force }
        $hr3 = -1
        try { $hr3 = [PXD]::Extract($testPsd, 256, $png) } catch {}
        $v3 = if($hr3 -eq 0 -and (Test-Path $png)){'SUCCESS after relocation'}else{'still failing - SEND REPORT BACK'}
        Log ("  retest hr = 0x{0:X8}  {1}" -f $hr3, $v3)
      }
    }
  } else { Log "  fix skipped: install dir or dll not found" }
} else {
  Log ""
  Log "Tip: rerun with -Fix to auto-repair registration."
}

$out = Join-Path ([Environment]::GetFolderPath('Desktop')) 'PixLens-thumb-diag.txt'
$report | Out-File -FilePath $out -Encoding UTF8
Write-Host ""
Write-Host "Report saved: $out"
