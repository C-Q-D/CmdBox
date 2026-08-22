@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\restart.ps1" %*
set "cmdBoxExitCode=%ERRORLEVEL%"
exit /b %cmdBoxExitCode%
REM 本入口精确重启当前仓库记录的后台 Windows Tauri Dev。
