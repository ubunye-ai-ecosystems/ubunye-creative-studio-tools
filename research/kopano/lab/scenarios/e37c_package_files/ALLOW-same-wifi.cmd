@echo off
rem Kopano test: let teammates on the SAME Wi-Fi reach this test program directly.
rem One Windows Firewall rule, only for peer.exe in this folder, only UDP, only from the local
rem network. Undo with REMOVE-same-wifi.cmd. Windows asks for administrator approval.
cd /d "%~dp0"
if not exist "%~dp0peer.exe" (
  echo This is running from inside the zip file.
  echo Please right-click the zip, choose Extract All, then run this file from the extracted folder.
  pause
  exit /b 1
)
net session >nul 2>&1
if errorlevel 1 (
  echo Windows will now ask for permission. Click Yes.
  powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -Verb RunAs"
  exit /b 0
)
netsh advfirewall firewall delete rule name="Kopano test (same Wi-Fi)" >nul 2>&1
netsh advfirewall firewall add rule name="Kopano test (same Wi-Fi)" dir=in action=allow program="%~dp0peer.exe" protocol=UDP remoteip=localsubnet profile=any enable=yes
if errorlevel 1 (
  echo Could not add the rule. Please tell Thabang.
) else (
  echo Done. Teammates on the same Wi-Fi can now reach the Kopano test.
)
pause
