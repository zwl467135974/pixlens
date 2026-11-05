# Dump BagMRU root + first-level children item blobs (ASCII-safe output)
function DumpNode($key, $label, $max) {
  Write-Host "== $label =="
  $p = Get-ItemProperty $key -ErrorAction SilentlyContinue
  $props = @($p.PSObject.Properties | Where-Object { $_.Name -match '^\d+$' })
  for ($i = 0; $i -lt $props.Count -and $i -lt $max; $i++) {
    $b = $props[$i].Value
    $ascii = -join ($b | ForEach-Object { if ($_ -ge 0x20 -and $_ -le 0x7E) { [char]$_ } else { '.' } })
    $hex = ($b[0..([Math]::Min(47, $b.Length-1))] | ForEach-Object { $_.ToString('X2') }) -join ''
    Write-Host (" [" + $props[$i].Name + "] len=" + $b.Length + " ascii='" + $ascii + "'")
    Write-Host ("   hex " + $hex)
  }
}
DumpNode 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU' 'ROOT' 12
foreach ($sub in Get-ChildItem 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU' -ErrorAction SilentlyContinue) {
  if ($sub.PSChildName -match '^\d+$') {
    $kids = @(Get-ChildItem $sub.PSPath -ErrorAction SilentlyContinue)
    if ($kids.Count -gt 0) {
      DumpNode $sub.PSPath ("child " + $sub.PSChildName) 10
    }
  }
}
