param()
$ErrorActionPreference = 'Stop'
$token = '3bf2e142eb65e3b249e1bf4154d8c909'
$repo = 'wyler_admin/pixlens'
$tag = 'v1.4.0'
$name = 'PixLens v1.4.0'
# ASCII-safe body (Gitee API encoding); CRLF to keep markdown line breaks
$body = "## RAW camera files (19 formats)`r`n`r`n- CR2 / CR3 / NEF / ARW / DNG / ORF / RW2 / RAF / PEF / SRW / X3F / ERF / MRW / KDC / DCR / IIQ / RWL / GPR / 3FR are now viewable without camera vendor software`r`n- Decoded with libraw-wasm in a Web Worker (1.4MB wasm loaded on first RAW file - zero cost at startup)`r`n- Camera white balance + camera color matrix + sRGB 8-bit output; thumbnails use halfSize decoding (~4x faster)`r`n- Rust side only passes raw bytes through with disk-read throttling (~6ms for a 10MB file)`r`n`r`n## Default image viewer (one click)`r`n`r`n- New section in Settings: register/unregister as the system image viewer candidate (33 image formats incl. PSD/RAW)`r`n- Register: HKCU ProgId + Open With list + Windows Default Apps entry; original .ext defaults are backed up and fully restored on unregister`r`n- Set as default opens Windows Settings (ms-settings:defaultapps) for one-click confirmation - same flow as Chrome/Firefox (Windows 10/11 requires user confirmation)`r`n- Uninstall cleans up all association registrations`r`n`r`nNote: after unregistering, Windows picks a new default app automatically (it cannot be programmatically rolled back to the exact previous default - Windows limitation)."

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
$exe = 'D:\work\pixlens\target\release\bundle\nsis\PixLens_1.4.0_x64-setup.exe'
$boundary = '----pixlensboundary' + [DateTime]::Now.Ticks
$uri = "https://gitee.com/api/v5/repos/$repo/releases/$id/attach_files?access_token=$token"
$req2 = [System.Net.HttpWebRequest]::Create($uri)
$req2.Method = 'POST'
$req2.ContentType = "multipart/form-data; boundary=$boundary"
$fs = [IO.File]::OpenRead($exe)
$head = ("--$boundary`r`nContent-Disposition: form-data; name=`"file`"; filename=`"PixLens_1.4.0_x64-setup.exe`"`r`nContent-Type: application/octet-stream`r`n`r`n").Replace('\"','"')
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
