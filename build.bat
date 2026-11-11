@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem The current OS/CPU builds directly with cargo. Other targets, cross-rs and
rem the interactive menu share build.sh with macOS/Linux through Git for Windows.
call "%~dp0build\windows-env.bat"
if errorlevel 1 exit /b %errorlevel%
set "NABI_PAUSE="
rem Keep a double-clicked build's result visible.
if "%~1"=="" set "NABI_PAUSE=1"

where rustc >nul 2>nul
if errorlevel 1 goto shell
set "NABI_HOST="
pushd "%~dp0"
for /f "tokens=2" %%H in ('rustc -vV ^| findstr /b /c:"host:"') do set "NABI_HOST=%%H"
popd
if not defined NABI_HOST goto shell

set "NABI_ACTION="
set "NABI_DRY_RUN="
:parse
if "%~1"=="" goto native
if /i "%~1"=="native" goto next
if /i "%~1"=="%NABI_HOST%" goto next
if /i "%~1"=="--target" goto parse_target
if /i "%~1"=="--full" goto action_full
if /i "%~1"=="--dmg" goto action_full
if /i "%~1"=="--binary" goto action_binary
if /i "%~1"=="--package" goto action_package
if /i "%~1"=="--package-only" goto action_package_only
if /i "%~1"=="--dry-run" (set "NABI_DRY_RUN=1" & goto next)
if /i "%~1"=="-y" goto next
if /i "%~1"=="--yes" goto next
rem Other targets and options (help, --cross, --interactive, ...) use build.sh.
goto shell
:parse_target
if /i "%~2"=="native" (shift & goto next)
if /i "%~2"=="%NABI_HOST%" (shift & goto next)
goto shell
:action_full
set "NABI_NEXT=full"
goto set_action
:action_binary
set "NABI_NEXT=binary"
goto set_action
:action_package
rem Windows packages are ZIP files, so --package matches --full.
set "NABI_NEXT=full"
goto set_action
:action_package_only
set "NABI_NEXT=package-only"
:set_action
rem build.sh reports conflicting actions.
if defined NABI_ACTION goto shell
set "NABI_ACTION=%NABI_NEXT%"
:next
shift
goto parse

:native
if not defined NABI_ACTION set "NABI_ACTION=full"
pushd "%~dp0"
echo Host: %NABI_HOST%
echo Target: %NABI_HOST%
echo Action: %NABI_ACTION%
echo Builder: cargo (direct)
if defined NABI_DRY_RUN goto dry_run

call :need cargo || goto failed
if "%NABI_ACTION%"=="package-only" goto check_python
call :need cmake || goto failed
call :need perl || goto failed
if "%NABI_HOST:~0,7%"=="x86_64-" call :need nasm || goto failed
if "%NABI_ACTION%"=="binary" goto compile
:check_python
call :find_python || goto failed
rem notices.py inventories all locked platforms, not just the selected target.
cargo fetch --locked || goto failed
if "%NABI_ACTION%"=="package-only" goto package
:compile
cargo build --locked --release --bin nabi-view --target %NABI_HOST% || goto failed
if "%NABI_ACTION%"=="binary" goto binary_done
:package
%NABI_PYTHON% build\notices.py || goto failed
%NABI_PYTHON% build\package-portable.py --target %NABI_HOST% || goto failed
set "NABI_RESULT=0"
goto done
:binary_done
echo Build complete. Binary: ^<Cargo target directory^>/%NABI_HOST%/release/nabi-view.exe
set "NABI_RESULT=0"
goto done

:dry_run
if not "%NABI_ACTION%"=="binary" echo   cargo fetch --locked
if not "%NABI_ACTION%"=="package-only" echo   cargo build --locked --release --bin nabi-view --target %NABI_HOST%
if "%NABI_ACTION%"=="binary" goto dry_run_done
echo   python build\notices.py
echo   python build\package-portable.py --target %NABI_HOST%
echo Output: ^<Cargo target directory^>/package/%NABI_HOST%/
:dry_run_done
set "NABI_RESULT=0"
goto done

:failed
set "NABI_RESULT=1"
:done
popd
goto finish

:shell
set "NABI_SH=%ProgramFiles%\Git\bin\sh.exe"
if exist "%NABI_SH%" goto run_shell
set "NABI_SH=%LocalAppData%\Programs\Git\bin\sh.exe"
if exist "%NABI_SH%" goto run_shell
where sh.exe >nul 2>nul
if errorlevel 1 (
    echo Git Bash was not found. Install Git for Windows. 1>&2
    set "NABI_RESULT=1"
    goto finish
)
set "NABI_SH=sh.exe"
:run_shell
"%NABI_SH%" "%~dp0build.sh" %*
set "NABI_RESULT=%errorlevel%"

:finish
if defined NABI_PAUSE pause
exit /b %NABI_RESULT%

:need
where %1 >nul 2>nul
if not errorlevel 1 exit /b 0
echo Error: %1 was not found. Install it and add its executable directory to PATH. 1>&2
exit /b 1

:find_python
rem Packaging requires Python 3.9+ (override with PYTHON=C:\path\to\python.exe).
if defined PYTHON (set NABI_PYTHON="%PYTHON%"& goto check_version)
set "NABI_PYTHON=python"
call :python_ok && exit /b 0
set "NABI_PYTHON=py -3"
call :python_ok && exit /b 0
echo Error: Python 3.9+ is required for packaging. Set PYTHON to its executable path. 1>&2
exit /b 1
:check_version
call :python_ok && exit /b 0
echo Error: Python 3.9+ is required. 1>&2
exit /b 1

:python_ok
%NABI_PYTHON% -c "import sys; sys.exit(sys.version_info < (3, 9))" >nul 2>nul
exit /b %errorlevel%
