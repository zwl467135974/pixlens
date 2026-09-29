$psd = Get-Item "C:\Users\GA\Desktop\PSD*\small_rgb8_rle.psd" | Select-Object -First 1
$psd.LastWriteTime = Get-Date
Write-Output ("TOUCHED " + $psd.FullName)
