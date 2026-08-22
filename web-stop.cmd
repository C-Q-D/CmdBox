@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\web-stop.ps1" %*
set "cmdBoxExitCode=%ERRORLEVEL%"
exit /b %cmdBoxExitCode%
REM 本入口精确停止当前仓库的 Docker Compose Watch 与前端容器。
