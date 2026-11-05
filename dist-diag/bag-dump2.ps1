# Dump Desktop bag Sort blob + walk BagMRU to find D:\PSD test folder bag (ASCII only)
$src = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\Shell\Bags\1\Desktop' -ErrorAction SilentlyContinue
if ($src -and $src.Sort) {
  $b = $src.Sort
  Write-Host ("DESKTOP Sort len=" + $b.Length)
  Write-Host ((($b | ForEach-Object { $_.ToString('X2') }) -join ' '))
}

# Walk BagMRU: find numeric subkeys with binary values, show NodeSlot mapping depth 2
$mru = 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU'
function Dump-Node($key, $depth) {
  if ($depth -gt 2) { return }
  $p = Get-ItemProperty $key -ErrorAction SilentlyContinue
  if ($p -and $p.PSObject.Properties.Name -contains 'NodeSlot') {
    Write-Host ("  " * $depth + "[slot " + $p.NodeSlot + "]")
  }
  foreach ($prop in ($p.PSObject.Properties | Where-Object { $_.Name -match '^\d+$' })) {
    $bytes = $prop.Value
    # extract utf16 runs
    $s = [Text.Encoding]::Unicode.GetString($bytes) -replace '[^\x20-\x7E\u4e00-\u9fff]', '.'
    Write-Host ("  " * $depth + "item[" + $prop.Name + "] len=" + $bytes.Length + " name~" + $s.Trim('.'))
  }
  foreach ($sub in Get-ChildItem $key -ErrorAction SilentlyContinue) {
    if ($sub.PSChildName -match '^\d+$') { Dump-Node $sub.PSPath ($depth + 1) }
  }
}
if (Test-Path $mru) { Dump-Node $mru 0 } else { Write-Host "NO MRU" }
