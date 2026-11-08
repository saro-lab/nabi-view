@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Share the same build menu with macOS/Linux through Git for Windows.
call "%~dp0build\windows-env.bat"
if errorlevel 1 exit /b %errorlevel%
set "NABI_SH=%ProgramFiles%\Git\bin\sh.exe"
if exist "%NABI_SH%" goto build
set "NABI_SH=%LocalAppData%\Programs\Git\bin\sh.exe"
if exist "%NABI_SH%" goto build
where sh.exe >nul 2>nul
if errorlevel 1 (
    echo Git Bash was not found. Install Git for Windows. 1>&2
    exit /b 1
)
set "NABI_SH=sh.exe"
:build
"%NABI_SH%" "%~dp0build.sh" %*
set "NABI_RESULT=%errorlevel%"
rem Keep a double-clicked interactive build's result visible.
if "%~1"=="" pause
exit /b %NABI_RESULT%
