Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class QiProbe {
    [DllImport("ole32.dll")]
    private static extern int CoCreateInstance(ref Guid clsid, IntPtr outer, uint ctx, ref Guid iid, out IntPtr obj);
    private static string Qi(IntPtr unk, Guid g, string name) {
        IntPtr p;
        int hr = Marshal.QueryInterface(unk, ref g, out p);
        if (p != IntPtr.Zero) Marshal.Release(p);
        return " | QI " + name + " hr=0x" + hr.ToString("X8");
    }
    public static string Run() {
        Guid clsid = new Guid("2B2E7C27-BC52-4521-9A56-87BC2DFC7639");
        Guid iidUnk = new Guid("00000000-0000-0000-C000-000000000046");
        Guid iidThumb = new Guid("E357FCCD-A995-4576-B01F-234630154E96");
        Guid iidStream = new Guid("B824B49D-22AC-4161-AC8A-9926BDFAFBD0");
        IntPtr unk;
        int hr = CoCreateInstance(ref clsid, IntPtr.Zero, 1, ref iidUnk, out unk);
        if (hr != 0) return "CoCreate hr=0x" + hr.ToString("X8");
        string r = "CoCreate OK" + Qi(unk, iidThumb, "Thumb") + Qi(unk, iidStream, "Stream");
        Marshal.Release(unk);
        return r;
    }
}
"@
Write-Output ([QiProbe]::Run())
