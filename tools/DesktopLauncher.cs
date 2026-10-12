using System;
using System.Diagnostics;
using System.IO;
using System.Windows.Forms;

internal static class DesktopLauncher
{
    [STAThread]
    private static int Main(string[] args)
    {
        string root = AppDomain.CurrentDomain.BaseDirectory;
        string python = Path.Combine(root, "build", "runtime", "python312", "pythonw.exe");
        string entry = Path.Combine(root, "tools", "minihou_app.py");
        try
        {
            if (!File.Exists(python) || !File.Exists(entry))
                throw new FileNotFoundException("Keep MiniHou.exe in the MiniHou folder alongside its build and tools folders.");
            if (args.Length == 1 && args[0] == "--check") return 0;
            Process.Start(new ProcessStartInfo {
                FileName = python,
                Arguments = "\"" + entry + "\" app --workspace \"" + root.TrimEnd(Path.DirectorySeparatorChar) + "\"",
                WorkingDirectory = root,
                UseShellExecute = false,
                CreateNoWindow = true
            });
            return 0;
        }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "MiniHou could not start", MessageBoxButtons.OK, MessageBoxIcon.Error);
            return 1;
        }
    }
}
