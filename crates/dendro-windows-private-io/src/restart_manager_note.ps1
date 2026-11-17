param([Parameter(Mandatory = $true)][string]$Path)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class DendroRestartManagerNote {
    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    public static extern int RmStartSession(out uint session, uint flags, StringBuilder key);
    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    public static extern int RmRegisterResources(uint session, uint nFiles, string[] files, uint nApps, IntPtr apps, uint nServices, IntPtr services);
    [DllImport("rstrtmgr.dll", EntryPoint = "RmGetList")]
    public static extern int RmGetListProbe(uint session, out uint needed, ref uint count, IntPtr apps, out uint reasons);
    [DllImport("rstrtmgr.dll", EntryPoint = "RmGetList")]
    public static extern int RmGetList(uint session, out uint needed, ref uint count, [In, Out] RM_PROCESS_INFO[] apps, out uint reasons);
    [DllImport("rstrtmgr.dll")]
    public static extern int RmEndSession(uint session);
    [StructLayout(LayoutKind.Sequential)]
    public struct FILETIME { public uint Low; public uint High; }
    [StructLayout(LayoutKind.Sequential)]
    public struct RM_UNIQUE_PROCESS { public uint ProcessId; public FILETIME Started; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct RM_PROCESS_INFO {
        public RM_UNIQUE_PROCESS Process;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string AppName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string Service;
        public int ApplicationType;
        public uint AppStatus;
        public uint SessionId;
        public int Restartable;
    }
    public static string Note(string path) {
        var key = new StringBuilder(64);
        uint session;
        int rc = RmStartSession(out session, 0, key);
        if (rc != 0) return "restart-manager: unavailable error=" + rc;
        try {
            rc = RmRegisterResources(session, 1, new[] { path }, 0, IntPtr.Zero, 0, IntPtr.Zero);
            if (rc != 0) return "restart-manager: unavailable error=" + rc;
            uint needed = 0, count = 0, reasons;
            rc = RmGetListProbe(session, out needed, ref count, IntPtr.Zero, out reasons);
            if (rc == 0 && needed == 0) return "restart-manager: none_reported";
            if (rc != 0 && rc != 234) return "restart-manager: unavailable error=" + rc;
            if (needed == 0 || needed > 16) return "restart-manager: truncated needed=" + needed;
            var infos = new RM_PROCESS_INFO[needed];
            count = needed;
            rc = RmGetList(session, out needed, ref count, infos, out reasons);
            if (rc != 0) return "restart-manager: unavailable error=" + rc;
            if (count == 0) return "restart-manager: none_reported";
            var lines = new StringBuilder("restart-manager:");
            uint shown = count > (uint)infos.Length ? (uint)infos.Length : count;
            for (uint i = 0; i < shown; i++) {
                var info = infos[i];
                lines.Append(" pid=").Append(info.Process.ProcessId);
                lines.Append(" app=").Append(Clean(info.AppName));
                lines.Append(" service=").Append(Clean(info.Service));
                lines.Append(" type=").Append(TypeName(info.ApplicationType));
                lines.Append(" status=").Append(info.AppStatus);
                lines.Append(" session=").Append(info.SessionId);
                lines.Append(" restartable=").Append(info.Restartable == 0 ? 0 : 1);
                if (i + 1 < shown) lines.Append(" ;");
            }
            return lines.ToString();
        } finally {
            RmEndSession(session);
        }
    }
    static string Clean(string value) {
        if (string.IsNullOrEmpty(value)) return "";
        var builder = new StringBuilder(value.Length);
        foreach (char c in value) builder.Append(c < 32 || c == 127 ? '?' : c);
        return builder.ToString();
    }
    static string TypeName(int kind) {
        switch (kind) {
            case 0: return "unknown";
            case 1: return "main-window";
            case 2: return "other-window";
            case 3: return "service";
            case 4: return "explorer";
            case 5: return "console";
            case 1000: return "critical";
            default: return "type-" + kind;
        }
    }
}
'@
[DendroRestartManagerNote]::Note($Path)
