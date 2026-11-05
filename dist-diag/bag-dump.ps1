# Dump shellbag structure: value names + sample Sort blobs (ASCII only)
$roots = @(
  'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags',
  'HKCU:\Software\Microsoft\Windows\Shell\Bags'
)
foreach ($root in $roots) {
  if (-not (Test-Path $root)) { continue }
  Write-Host "=== $root ==="
  $names = @{}
  $sortSamples = 0
  foreach ($bag in Get-ChildItem $root) {
    foreach ($sub in Get-ChildItem $bag.PSPath -ErrorAction SilentlyContinue) {
      $p = Get-ItemProperty $sub.PSPath -ErrorAction SilentlyContinue
      if ($p) {
        foreach ($k in $p.PSObject.Properties.Name) {
          if ($k.StartsWith('PS')) { continue }
          $key = $sub.PSChildName + '\' + $k
          if ($names.ContainsKey($key)) { $names[$key]++ } else { $names[$key] = 1 }
        }
      }
    }
  }
  $names.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 25 | ForEach-Object {
    Write-Host ("  " + $_.Key + " x" + $_.Value)
  }
}
