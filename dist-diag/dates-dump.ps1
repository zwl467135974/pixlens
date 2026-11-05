# Dump creation vs modified times for the test folder
$dir = 'D:\PSD缩略图测试'
Get-ChildItem -LiteralPath $dir -File | ForEach-Object {
  [PSCustomObject]@{
    Name = $_.Name
    Created = $_.CreationTime.ToString('MM-dd HH:mm:ss')
    Modified = $_.LastWriteTime.ToString('MM-dd HH:mm:ss')
  }
} | Sort-Object Modified | Format-Table -AutoSize | Out-String -Width 200
