# Find the "modified date" column index and show its value for the NEF
$app = New-Object -ComObject Shell.Application
$folder = $app.Namespace('D:\PSD缩略图测试')
for ($i = 0; $i -lt 20; $i++) {
  $h = $folder.GetDetailsOf($null, $i)
  if ($h) { Write-Host ("col " + $i + ": " + $h) }
}
Write-Host '--- NEF row ---'
$nef = $folder.Items() | Where-Object { $_.Name -like '*D90*' }
for ($i = 0; $i -lt 20; $i++) {
  $v = $folder.GetDetailsOf($nef, $i)
  if ($v) { Write-Host ("  col " + $i + ": " + $v) }
}
