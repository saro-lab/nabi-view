@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Windows counterpart of mobile.sh. The mobile preview runs directly with
rem cargo; Android shares build/android.sh with macOS/Linux through Git for Windows.
call "%~dp0build\windows-env.bat"
if errorlevel 1 exit /b %errorlevel%
set "NABI_PAUSE="
rem Keep a double-clicked run's help visible.
if "%~1"=="" set "NABI_PAUSE=1"
set "NABI_PLATFORM=%~1"
if not defined NABI_PLATFORM set "NABI_PLATFORM=--help"
rem Forward the remaining arguments unchanged.
set "NABI_ARGS="
:collect
shift
if "%~1"=="" goto dispatch
set NABI_ARGS=%NABI_ARGS% %1
goto collect

:dispatch
if /i "%NABI_PLATFORM%"=="preview" goto preview
if /i "%NABI_PLATFORM%"=="android" goto android
if /i "%NABI_PLATFORM%"=="ios" goto ios
if /i "%NABI_PLATFORM%"=="iphone" goto ios
if /i "%NABI_PLATFORM%"=="ipad" goto ios
if /i "%NABI_PLATFORM%"=="-h" goto help
if /i "%NABI_PLATFORM%"=="--help" goto help
echo Unknown platform: %NABI_PLATFORM% 1>&2
set "NABI_RESULT=2"
goto finish

:preview
where cargo >nul 2>nul
if errorlevel 1 (
    echo Error: cargo was not found. Install Rust and add it to PATH. 1>&2
    set "NABI_RESULT=1"
    goto finish
)
set "NABI_HOST="
pushd "%~dp0"
for /f "tokens=2" %%H in ('rustc -vV ^| findstr /b /c:"host:"') do set "NABI_HOST=%%H"
popd
if not defined NABI_HOST (
    echo Error: could not determine the Rust host target. 1>&2
    set "NABI_RESULT=1"
    goto finish
)
pushd "%~dp0"
rem Same target directory as build.bat, so its compiled C dependencies are reused.
cargo run --locked --release --bin nabi-view --target %NABI_HOST% -- --mobile-preview%NABI_ARGS%
set "NABI_RESULT=%errorlevel%"
popd
goto finish

:android
if defined ANDROID_HOME goto find_shell
if defined ANDROID_SDK_ROOT goto find_shell
if exist "%LocalAppData%\Android\Sdk" set "ANDROID_HOME=%LocalAppData%\Android\Sdk"
:find_shell
set "NABI_SH=%ProgramFiles%\Git\bin\sh.exe"
if exist "%NABI_SH%" goto run_android
set "NABI_SH=%LocalAppData%\Programs\Git\bin\sh.exe"
if exist "%NABI_SH%" goto run_android
where sh.exe >nul 2>nul
if errorlevel 1 (
    echo Git Bash was not found. Install Git for Windows. 1>&2
    set "NABI_RESULT=1"
    goto finish
)
set "NABI_SH=sh.exe"
:run_android
"%NABI_SH%" "%~dp0build\android.sh"%NABI_ARGS%
set "NABI_RESULT=%errorlevel%"
goto finish

:ios
echo iPhone/iPad builds need macOS with Xcode. On a Mac, run: ./mobile.sh ios or ./mobile.sh ipad 1>&2
set "NABI_RESULT=2"
goto finish

:help
echo NABI VIEW mobile
echo   mobile.bat preview               Check the phone/tablet layout on this PC
echo   mobile.bat android               Build, install and run on a connected phone/tablet/emulator
echo   mobile.bat android --build-only  Build the shared phone/tablet APK
echo   mobile.bat android --release --build-only  Build a release APK
echo   mobile.bat android --release --bundle      Build a Play AAB
echo   mobile.bat android --release --bundle --abi all  Include both 64-bit ABIs
echo   mobile.bat android --abi x86_64  Build for an x86_64 emulator
echo   mobile.bat android --serial ID   Run on a specific adb device
echo   iPhone/iPad need macOS and Xcode: use ./mobile.sh ios or ./mobile.sh ipad on a Mac.
echo.
echo Android needs Android Studio's SDK, NDK and CMake (for Ninja), Gradle 8.11.1, JDK 17
echo and the Rust target (rustup target add aarch64-linux-android).
echo ANDROID_HOME defaults to %%LOCALAPPDATA%%\Android\Sdk.
set "NABI_RESULT=0"

:finish
if defined NABI_PAUSE pause
exit /b %NABI_RESULT%
