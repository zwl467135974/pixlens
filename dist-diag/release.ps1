param()
$ErrorActionPreference = 'Stop'
$token = '3bf2e142eb65e3b249e1bf4154d8c909'
$repo = 'wyler_admin/pixlens'
$tag = 'v1.4.2'
$name = 'PixLens v1.4.2'
# ASCII-safe body (Gitee API encoding); CRLF to keep markdown line breaks
$body = "## Multi-folder merged browsing`r`n`r`n- New 'Add Folder' button (Ctrl+Shift+O) appends a second folder's images into the same grid - cross-folder compare (select two, press C), paging, slideshow, search and batch tools all just work`r`n- Source folder chips below the toolbar: hover for full path, X to remove`r`n- New 'Folder' sort key keeps the two folders grouped; per-workset sort is remembered across restarts`r`n- Verified: appending a folder completes in 5-10ms; virtual scrolling and thumbnail LRU stay bounded regardless of total count`r`n`r`n## Grid zoom with Ctrl+wheel`r`n`r`n- Ctrl+wheel resizes thumbnails in 8px steps (same granularity as the slider), cursor-anchored so content does not jump`r`n- Ctrl+= / Ctrl+- / Ctrl+0 shortcuts; touchpad pinch works natively; two-way synced with the slider`r`n- Thumbnail size is remembered across restarts`r`n`r`nBoth features stay within the performance budget (bounded virtual scrolling + thumbnail LRU + throttled IO), bench pipeline regression passed."

[Console]::OutputEncoding = [Text.Encoding]::UTF8
$req = [System.Net.HttpWebRequest]::Create("https://gitee.com/api/v5/repos/$repo/releases")
$req.Method = 'POST'
$req.ContentType = 'application/json; charset=utf-8'
$req.Accept = 'application/json'
$payload = @{
    access_token = $token
    tag_name     = $tag
    name         = $name
    body         = $body
    target_commitish = 'main'
    prerelease   = $false
} | ConvertTo-Json
$bytes = [Text.Encoding]::UTF8.GetBytes($payload)
$req.ContentLength = $bytes.Length
$s = $req.GetRequestStream(); $s.Write($bytes, 0, $bytes.Length); $s.Close()
try {
    $resp = $req.GetResponse()
} catch [System.Net.WebException] {
    $r = $_.Exception.Response
    $sr = New-Object IO.StreamReader($r.GetResponseStream())
    Write-Output ("ERR " + [int]$r.StatusCode + " " + $sr.ReadToEnd())
    exit 1
}
$sr = New-Object IO.StreamReader($resp.GetResponseStream())
$text = $sr.ReadToEnd()
$id = ($text | ConvertFrom-Json).id
Write-Output ("CREATED id=" + $id)
$response = $text

# attach installer
$exe = 'D:\work\pixlens\target\release\bundle\nsis\PixLens_1.4.2_x64-setup.exe'
$boundary = '----pixlensboundary' + [DateTime]::Now.Ticks
$uri = "https://gitee.com/api/v5/repos/$repo/releases/$id/attach_files?access_token=$token"
$req2 = [System.Net.HttpWebRequest]::Create($uri)
$req2.Method = 'POST'
$req2.ContentType = "multipart/form-data; boundary=$boundary"
$fs = [IO.File]::OpenRead($exe)
$head = ("--$boundary`r`nContent-Disposition: form-data; name=`"file`"; filename=`"PixLens_1.4.2_x64-setup.exe`"`r`nContent-Type: application/octet-stream`r`n`r`n").Replace('\"','"')
$headBytes = [Text.Encoding]::UTF8.GetBytes($head)
$tailBytes = [Text.Encoding]::UTF8.GetBytes("`r`n--$boundary--`r`n")
$req2.ContentLength = $headBytes.Length + $fs.Length + $tailBytes.Length
$up = $req2.GetRequestStream()
$up.Write($headBytes, 0, $headBytes.Length)
$buf = New-Object byte[] (64 * 1024)
while (($n = $fs.Read($buf, 0, $buf.Length)) -gt 0) { $up.Write($buf, 0, $n) }
$up.Write($tailBytes, 0, $tailBytes.Length)
$up.Close(); $fs.Close()
try { $resp2 = $req2.GetResponse() } catch [System.Net.WebException] {
    $r2 = $_.Exception.Response
    $sr2 = New-Object IO.StreamReader($r2.GetResponseStream())
    Write-Output ("ATTACH_ERR " + [int]$r2.StatusCode + " " + $sr2.ReadToEnd())
    exit 1
}
$sr2 = New-Object IO.StreamReader($resp2.GetResponseStream())
Write-Output ("ATTACHED " + $sr2.ReadToEnd())
