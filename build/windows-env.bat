@echo off
rem Called inside the launcher's SETLOCAL: never change the user's saved PATH.
rem Prefer PATH tools, then common installs, then this checkout's local tools.
call :add_tool cmake "%ProgramFiles%\CMake\bin"
call :add_tool cmake "%LocalAppData%\Programs\CMake\bin"
call :add_tool perl "%ProgramFiles%\Git\usr\bin"
call :add_tool perl "%LocalAppData%\Programs\Git\usr\bin"
for /f "delims=" %%G in ('where git.exe 2^>nul') do call :add_tool perl "%%~dpG..\usr\bin"
call :add_tool perl "C:\Strawberry\perl\bin"
call :add_tool nasm "%ProgramFiles%\NASM"
call :add_tool nasm "%LocalAppData%\bin\NASM"
call :add_tool nasm "%LocalAppData%\Programs\NASM"
for /d %%D in ("%~dp0..\ai\runtime\nasm-*") do call :add_tool nasm "%%~fD"
exit /b 0

:add_tool
where %~1.exe >nul 2>&1
if not errorlevel 1 exit /b 0
if exist "%~2\%~1.exe" set "PATH=%PATH%;%~2"
exit /b 0
