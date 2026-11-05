# Find every registry value named Sort under shell bag locations; hexdump non-trivial ones (ASCII only)
$paths = @(
  'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags',
  'HKCU:\Software\Microsoft\Windows\Shell\Bags',
  'HKCU:\Software\Microsoft\Windows\ShellNoROAM\Bags'
)
foreach ($root in $paths) {
  if (-not (Test-Path $root)) { continue }
  foreach ($item in Get-ChildItem $root -Recurse -ErrorAction SilentlyContinue) {
    $p = Get-ItemProperty $item.PSPath -ErrorAction SilentlyContinue
    if ($p -and ($p.PSObject.Properties.Name -contains 'Sort')) {
      $b = $p.Sort
      $hex = ($b | ForEach-Object { $_.ToString('X2') }) -join ''
      Write-Host ($item.PSPath.Replace('Microsoft.PowerShell.Core\Registry::HKEY_CURRENT_USER','HKCU') + ' len=' + $b.Length)
      Write-Host ('  ' + $hex)
    }
  }
}
# also AllFolders template
foreach ($t in @('HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags\AllFolders\Shell')) {
  if (Test-Path $t) {
    $p = Get-ItemProperty $t -ErrorAction SilentlyContinue
    if ($p) { Write-Host ($t + ' values: ' + (($p.PSObject.Properties.Name | Where-Object { -not $_.StartsWith('PS') }) -join ',')) }
  }
}
