param()
$ErrorActionPreference = 'Stop'
$token = '3bf2e142eb65e3b249e1bf4154d8c909'
$repo = 'wyler_admin/pixlens'
$tag = 'v1.3.2'
$name = 'PixLens v1.3.2'
# ASCII-safe body (Gitee API encoding); CRLF to keep markdown line breaks
$body = "## Explorer video thumbnails (shell extension)`r`n`r`n- MP4 / M4V / MOV / WebM / MKV / AVI / WMV now show a representative frame (1s) with a play badge in the thumbnail grid`r`n- Portrait rotation metadata is applied automatically`r`n- Double-click / Space opens the file with the system default player`r`n- Batch convert and Compare automatically skip videos`r`n- Thumbnails share the existing disk cache (~4ms warm)`r`n`r`nNote: Explorer thumbnails for MP4 are handled by Windows itself and are not affected."

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
$exe = 'D:\work\pixlens\target\release\bundle\nsis\PixLens_1.3.2_x64-setup.exe'
$boundary = '----pixlensboundary' + [DateTime]::Now.Ticks
$uri = "https://gitee.com/api/v5/repos/$repo/releases/$id/attach_files?access_token=$token"
$req2 = [System.Net.HttpWebRequest]::Create($uri)
$req2.Method = 'POST'
$req2.ContentType = "multipart/form-data; boundary=$boundary"
$fs = [IO.File]::OpenRead($exe)
$head = ("--$boundary`r`nContent-Disposition: form-data; name=`"file`"; filename=`"PixLens_1.3.2_x64-setup.exe`"`r`nContent-Type: application/octet-stream`r`n`r`n").Replace('\"','"')
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
