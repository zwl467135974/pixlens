# Dump shell DateTaken / DateModified per file via Shell COM
$app = New-Object -ComObject Shell.Application
$folder = $app.Namespace('D:\PSD缩略图测试')
foreach ($item in $folder.Items()) {
  $taken = $folder.GetDetailsOf($item, 12)   # 拍摄日期 column idx (varies, print raw)
  $modified = $item.ModifyDate
  Write-Host ($item.Name + ' | taken=' + $taken + ' | modified=' + $modified.ToString('MM-dd HH:mm:ss'))
}
