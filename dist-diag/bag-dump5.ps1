# List value names+types for key BagMRU nodes and sample bags (ASCII-safe)
function DumpVals($path, $label) {
  $k = Get-Item $path -ErrorAction SilentlyContinue
  if (-not $k) { Write-Host "$label MISSING"; return }
  Write-Host "== $label =="
  foreach ($vn in $k.GetValueNames()) {
    $t = $k.GetValueKind($vn)
    $v = $k.GetValue($vn)
    if ($v -is [byte[]]) {
      $hex = ($v[0..([Math]::Min(23, $v.Length-1))] | ForEach-Object { $_.ToString('X2') }) -join ''
      Write-Host ("  '" + $vn + "' type=" + $t + " len=" + $v.Length + " " + $hex)
    } else {
      Write-Host ("  '" + $vn + "' type=" + $t + " val=" + $v)
    }
  }
}
$base = 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell'
DumpVals "$base\Shell\BagMRU" 'BagMRU root'
DumpVals "$base\Shell\BagMRU\2" 'BagMRU\2 (Computer)'
DumpVals "$base\Shell\BagMRU\2\6" 'BagMRU\2\6 (D:)'
DumpVals "$base\Shell\Bags\41\Shell" 'Bags\41\Shell (root bag)'
DumpVals "$base\Shell\Bags\10\Shell" 'Bags\10\Shell (sample bag)'
