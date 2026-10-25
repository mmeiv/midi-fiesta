# <img src="src-tauri/icons/icon.png" width="40" alt=""> midi-fiesta

A small Windows app for making temporary MIDI connections between music programs, including Ableton Live.

## Download

Download the latest Windows installer from the [GitHub Releases](https://github.com/mmeiv/midi-fiesta/releases) page. Choose the `.exe` or `.msi` installer.

## MIDI backends

Choose the backend that is available on your PC:

- **teVirtualMIDI** uses the teVirtualMIDI driver. Install the driver and interface DLL separately; they are not included in this repository or the installer.
- **Windows MIDI Services** uses the virtual MIDI transport built into Windows. It needs Windows 11 version 25H2 or later, with Windows MIDI Services enabled.

Ports are removed when you delete them or close midi-fiesta. Changing backends closes the ports created by the current backend.

## Building on Windows

You'll need Windows 11, Node.js LTS, Rust with the MSVC toolchain, Microsoft C++ Build Tools (select **Desktop development with C++**) and the .NET 10 SDK.

The .NET SDK is only needed to build the app. The MIDI host is bundled with it, so users don't need to install .NET. Windows 11 includes the WebView2 runtime used by the app.

To run the app in development:

```powershell
npm ci
npm run tauri dev
```

To build the Windows installers:

```powershell
npm run tauri build
```

Installers are written to `src-tauri/target/release/bundle/`.

## License

midi-fiesta is released under the MIT License; see [LICENSE](LICENSE). The teVirtualMIDI SDK, driver and DLL are third-party components covered by separate license terms. The MIT License does not apply to them.
