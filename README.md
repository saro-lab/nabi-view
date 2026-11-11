# NABI VIEW

[https://lab.saro.me/app/nabi-view](https://lab.saro.me/app/nabi-view)

A fast, lightweight image viewer built with Rust.

- **Desktop:** Windows, macOS and Linux.
- **Mobile:** iOS and Android (in development).

## License

Free and open source under the [MIT License](LICENSE).
Third-party components retain their own licenses; see [Third-party notices](THIRD_PARTY.md).

## Build

Install Rust through rustup, plus Git, CMake, Perl and Python 3.9+.
Intel/x64 builds also require NASM. Get the source:

```sh
git clone https://github.com/saro-lab/nabi-view.git
cd nabi-view
```

Run `rustup show` from the repository root to install the Rust version and
components declared in `rust-toolchain.toml`. The target commands below use
that same toolchain. Run all build commands from the repository root.

### Windows

Install Visual Studio C++ Build Tools with the Windows SDK and Git for Windows.
Use the MSVC Rust toolchain, then run in PowerShell or CMD:

```powershell
.\build.bat native --full
```

Output: executable and ZIP in `target/package/<target>/`.

### macOS

Install Xcode Command Line Tools. With Homebrew:

```sh
xcode-select --install
brew install cmake nasm python
sh build.sh native --full
```

For a Universal build (Apple Silicon + Intel):

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
sh build.sh macos-universal --full
```

Output: `.app` in `target/package/<target>/` and `.dmg` in `target/dist/`.

### Linux

On Ubuntu / Debian, install the native build dependencies:

```sh
sudo apt-get update
sudo apt-get install -y build-essential cmake nasm perl pkg-config python3 \
  libxkbcommon-dev libwayland-dev libx11-dev libxcb1-dev libvulkan1
sh build.sh native --full
```

Output: executable and `.tar.gz` in `target/package/<target>/`.
Running the app requires a graphical desktop, GPU drivers and an appropriate
`xdg-desktop-portal` backend for file dialogs.

### iOS (development)

Requires macOS with Xcode and iOS platform support. Build for the iOS Simulator:

```sh
rustup target add aarch64-apple-ios-sim
sh build/ios.sh --build-only
```

On Intel Macs, install `x86_64-apple-ios` instead.
Output: `target/mobile/<target>/NABI VIEW.app`.

### Android (development)

On macOS or Linux, install JDK 17, Gradle 8.11.1, Android SDK Platform 36,
Build Tools 35 and an Android NDK (r28 or newer recommended by the build script).
Set the paths to your installed SDK and NDK, then build an ARM64 debug APK:

```sh
export ANDROID_HOME="/path/to/Android/sdk"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/<installed-version>"
rustup target add aarch64-linux-android
sh build/android.sh --build-only
```

Output: `target/android/gradle/app/outputs/apk/debug/app-debug.apk`.

## Main libraries

- **egui / eframe** — cross-platform UI, windows and input handling.
- **wgpu** — GPU rendering and texture uploads.
- **image / png / jxl-oxide / libavif / resvg** — raster image, JPEG XL, AVIF and SVG decoding.
- **moxcms** — ICC profile color conversion.
- **zip / sevenz-rust2** — ZIP and 7z archive reading.
- **SQLite (rusqlite) / Serde** — local history, bookmarks and JSON settings.
- **rfd / native OS APIs** — file dialogs, permissions, platform image codecs and system integration.
