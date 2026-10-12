Set shell = CreateObject("WScript.Shell")
Set files = CreateObject("Scripting.FileSystemObject")
root = files.GetParentFolderName(WScript.ScriptFullName)
shell.CurrentDirectory = root
shell.Run Chr(34) & root & "\build\runtime\python312\pythonw.exe" & Chr(34) & " " & Chr(34) & root & "\tools\minihou_app.py" & Chr(34) & " app --workspace " & Chr(34) & root & Chr(34), 0, False
