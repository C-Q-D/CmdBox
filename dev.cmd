@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\dev.ps1" %*
set "cmdBoxExitCode=%ERRORLEVEL%"
exit /b %cmdBoxExitCode%
REM 本入口启动 CmdBox 完整 Windows Tauri 增量开发，可追加 -Detached 在后台运行。
