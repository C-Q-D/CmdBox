@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\web-dev.ps1" %*
set "cmdBoxExitCode=%ERRORLEVEL%"
exit /b %cmdBoxExitCode%
REM 本入口启动 CmdBox Docker 纯前端开发，可追加 -Detached 在后台运行。
