@echo off
rem Instalador do tkd: duplo clique aqui. Pode rodar de novo quando quiser (pula o que ja tem).
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\install.ps1" %*
echo.
pause
