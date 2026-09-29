$clsid = [Guid]"2B2E7C27-BC52-4521-9A56-87BC2DFC7639"
$type = [Type]::GetTypeFromCLSID($clsid)
if (-not $type) { Write-Output "TYPE_NULL"; exit 1 }
try { $obj = [Activator]::CreateInstance($type) } catch { Write-Output ("CREATE_FAIL " + $_.Exception.Message); exit 1 }
$unk = [Runtime.InteropServices.Marshal]::GetIUnknownForObject($obj)
foreach ($pair in @(
    @{ n = "IThumbnailProvider"; g = "E357FCCD-A995-4576-B01F-234630154E96" },
    @{ n = "IInitializeWithStream"; g = "B824B49D-22AC-4161-AC8A-9926BDFAFBD0" }
)) {
    $ptr = [IntPtr]::Zero
    $hr = [Runtime.InteropServices.Marshal]::QueryInterface($unk, [Guid]$pair.g, [ref]$ptr)
    Write-Output ("QI " + $pair.n + " hr=0x" + $hr.ToString("X8"))
    if ($ptr -ne [IntPtr]::Zero) { [Runtime.InteropServices.Marshal]::Release($ptr) | Out-Null }
}
[Runtime.InteropServices.Marshal]::Release($unk) | Out-Null
