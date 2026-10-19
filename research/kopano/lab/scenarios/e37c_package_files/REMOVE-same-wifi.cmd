@echo off
rem Kopano test: remove the firewall rule that ALLOW-same-wifi.cmd added.
net session >nul 2>&1
if errorlevel 1 (
  echo Windows will now ask for permission. Click Yes.
  powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -Verb RunAs"
  exit /b 0
)
netsh advfirewall firewall delete rule name="Kopano test (same Wi-Fi)"
echo Done. The Kopano test rule is removed.
pause
