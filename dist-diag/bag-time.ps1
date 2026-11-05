# Check shellbag node last-write times (ASCII-safe output)
$k = Get-Item -LiteralPath 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU\2\6' -ErrorAction SilentlyContinue
if ($k) { Write-Host ('D-drive node lastWrite: ' + $k.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss')) }
else { Write-Host 'D-drive node MISSING' }
$k2 = Get-Item -LiteralPath 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU\2\6\58' -ErrorAction SilentlyContinue
if ($k2) { Write-Host ('psd-test folder node lastWrite: ' + $k2.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss')) }
else { Write-Host 'psd-test node: not found (index may have shifted)' }
Write-Host ('now: ' + (Get-Date).ToString('yyyy-MM-dd HH:mm:ss'))
