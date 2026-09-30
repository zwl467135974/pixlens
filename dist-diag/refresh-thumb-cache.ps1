param()
$ErrorActionPreference = 'SilentlyContinue'
$dir = "$env:LOCALAPPDATA\Microsoft\Windows\Explorer"
$before = (Get-ChildItem $dir -Filter "thumbcache_*.db" | Measure-Object Length -Sum).Sum / 1MB
Stop-Process -Name explorer -Force
Stop-Process -Name dllhost -Force
Start-Sleep -Seconds 2
$deleted = 0
Get-ChildItem $dir -Filter "thumbcache_*.db" | ForEach-Object {
    if (Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue) { } else { $deleted++ }
}
Get-ChildItem $dir -Filter "iconcache_*.db" | Remove-Item -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
$left = (Get-ChildItem $dir -Filter "thumbcache_*.db" -ErrorAction SilentlyContinue | Measure-Object).Count
Start-Process explorer.exe
Start-Sleep -Seconds 3
Write-Output ("thumbcache was {0:N0} MB, files left after clean: {1}" -f $before, $left)
Write-Output "explorer restarted; thumbnails will be rebuilt on demand"
