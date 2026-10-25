fn main() {
    println!("cargo:rerun-if-changed=midi-host/MidiHost.csproj");
    println!("cargo:rerun-if-changed=midi-host/Program.cs");
    println!("cargo:rerun-if-changed=midi-host/metadata/Windows.Devices.Midi2.winmd");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let runtime = match arch.as_str() {
            "x86_64" => "win-x64",
            "aarch64" => "win-arm64",
            _ => panic!("Unsupported Windows target architecture for MIDI host: {arch}"),
        };
        let manifest_dir = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
        let host_build_dir = manifest_dir.join("../.midi-fiesta-build/midi-host");
        let intermediate_dir = host_build_dir.join("obj");
        let managed_output_dir = host_build_dir.join("bin");
        let publish_dir = host_build_dir.join("publish/app");
        let host_executable = publish_dir.join("MidiHost.exe");
        std::fs::create_dir_all(&publish_dir).expect("Could not create the MIDI host publish directory");
        let intermediate_property = format!("-p:BaseIntermediateOutputPath={}/", intermediate_dir.display());
        let managed_output_property = format!("-p:BaseOutputPath={}/", managed_output_dir.display());
        let host_inputs = [
            manifest_dir.join("midi-host/MidiHost.csproj"),
            manifest_dir.join("midi-host/Program.cs"),
            manifest_dir.join("midi-host/metadata/Windows.Devices.Midi2.winmd"),
        ];
        let output_time = host_executable.metadata().and_then(|metadata| metadata.modified()).ok();
        let mut needs_publish = output_time.is_none_or(|output_time| {
            host_inputs.iter().any(|input| {
                input.metadata().and_then(|metadata| metadata.modified()).map_or(true, |input_time| input_time > output_time)
            })
        });

        if !needs_publish {
            needs_publish = !std::process::Command::new(&host_executable)
                .arg("--verify-bundle")
                .status()
                .is_ok_and(|status| status.success());
        }

        if needs_publish {
            let clean_status = std::process::Command::new("dotnet")
                .args([
                    "clean", "midi-host/MidiHost.csproj", "--configuration", "Release",
                    "--runtime", runtime, "--verbosity", "quiet",
                    &intermediate_property, &managed_output_property,
                ])
                .status()
                .expect("The .NET 10 SDK is required to clean the Windows MIDI host");
            if !clean_status.success() {
                panic!("Failed to clean the Windows MIDI host before publishing");
            }
            let status = std::process::Command::new("dotnet")
                .args([
                    "publish", "midi-host/MidiHost.csproj", "--configuration", "Release",
                    "--runtime", runtime, "--self-contained", "true", "--output", publish_dir.to_str().expect("Invalid MIDI host publish path"),
                    "--verbosity", "quiet",
                    &intermediate_property, &managed_output_property,
                ])
                .status()
                .expect("The .NET 10 SDK is required to build the Windows MIDI host");
            if !status.success() {
                panic!("Failed to publish Windows MIDI host");
            }
            let verification = std::process::Command::new(&host_executable)
                .arg("--verify-bundle")
                .status()
                .expect("Could not verify the published MIDI host bundle");
            if !verification.success() {
                panic!("The published MIDI host bundle failed its startup check");
            }
        }
    }
    tauri_build::build()
}
