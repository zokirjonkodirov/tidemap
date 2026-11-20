//! Rust-only build/distribution tooling. No JavaScript toolchain or shell scripts.
mod bench;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Native build: {error}");
            ExitCode::FAILURE
        }
    }
}
fn execute() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let action = args
        .first()
        .and_then(|argument| argument.to_str())
        .unwrap_or("help");
    if !["build", "check", "run", "package", "bench"].contains(&action) {
        println!(
            "cargo native build   Compile the native release application\ncargo native check   Compile-check the application\ncargo native run     Open the native application\ncargo native package Build a portable native distribution\ncargo native bench   Time parse, layout and scene build of the diagram engine\n\nNo tests are invoked by these commands."
        );
        return Ok(());
    }
    if action == "bench" {
        return bench::run();
    }
    let cargo = PathBuf::from(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")));
    let compiler = cargo
        .parent()
        .map(|parent| parent.join(if cfg!(windows) { "rustc.exe" } else { "rustc" }))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("rustc"));
    let sysroot = output(Command::new(&compiler).args(["--print", "sysroot"]))?;
    let version = output(Command::new(&compiler).arg("-vV"))?;
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("Compiler did not report its host")?
        .to_owned();
    let sysroot = PathBuf::from(sysroot.trim());
    let inherited = env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
    let mut paths = Vec::new();
    if host.ends_with("windows-gnu") {
        let tools = sysroot.join("lib/rustlib").join(&host).join("bin");
        let contained = tools.join("self-contained");
        if !contained.join("dlltool.exe").is_file() {
            return Err(
                "The GNU Rust toolchain is missing rust-mingw. Install its rust-mingw component."
                    .into(),
            );
        }
        // rustc's dlltool invokes `as` from its own directory. Recent GNU
        // toolchains ship dlltool without the assembler, so seed it from the
        // installed MinGW-w64 tools once. The copy is self-contained.
        if !contained.join("as.exe").is_file() {
            let scoop = PathBuf::from(env::var_os("USERPROFILE").unwrap_or_default())
                .join("scoop/apps/mingw/current/bin/as.exe");
            let assembler = inherited.iter().map(|directory| directory.join("as.exe")).chain(std::iter::once(scoop))
                .find(|path| path.is_file())
                .ok_or("The GNU toolchain has no assembler for dlltool. Install MinGW-w64 (for example `scoop install mingw`) and retry.")?;
            fs::copy(assembler, contained.join("as.exe"))?;
        }
        paths.push(contained);
        paths.push(tools);
    }
    paths.push(sysroot.join("bin"));
    if let Some(parent) = cargo.parent() {
        paths.push(parent.to_owned());
    }
    paths.extend(inherited);
    let native_path = env::join_paths(paths)?;
    let command = if action == "check" {
        "check"
    } else if action == "run" {
        "run"
    } else {
        "build"
    };
    let mut build = Command::new(&cargo);
    build.current_dir(&root).env("PATH", &native_path).args([
        command,
        "--release",
        "--locked",
        "-p",
        "tidemap",
    ]);
    if action == "run" && args.len() > 1 {
        build.arg("--").args(&args[1..]);
    }
    let status = build.status()?;
    if !status.success() {
        return Err(format!("Cargo {command} failed ({status})").into());
    }
    if action == "package" {
        package(&root, &sysroot, &host, &cargo, &native_path)?;
    }
    Ok(())
}
fn output(command: &mut Command) -> Result<String> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    Ok(String::from_utf8(output.stdout)?)
}
fn package(
    root: &Path,
    sysroot: &Path,
    host: &str,
    cargo: &Path,
    native_path: &OsString,
) -> Result<()> {
    let windows = host.contains("windows");
    let executable_name = if windows {
        "Tidemap.exe"
    } else {
        "tidemap"
    };
    let compiled_name = if windows {
        "tidemap.exe"
    } else {
        "tidemap"
    };
    let executable = fs::read(root.join("target/release").join(compiled_name))?;
    let imports = if windows {
        pe_imports(&executable)?
    } else {
        BTreeSet::new()
    };
    let folder_name = format!("Tidemap-0.3.0-{host}");
    let directory = root.join("release").join(&folder_name);
    fs::create_dir_all(&directory)?;
    let mut files = vec![(executable_name.to_owned(), executable)];
    for name in &imports {
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "libgcc_s_seh-1.dll" | "libwinpthread-1.dll" | "libstdc++-6.dll"
        ) {
            let candidates = [
                sysroot
                    .join("lib/rustlib")
                    .join(host)
                    .join("bin")
                    .join(name),
                sysroot.join("bin").join(name),
            ];
            let path = candidates
                .iter()
                .find(|path| path.is_file())
                .ok_or_else(|| {
                    format!(
                        "The executable needs {name}, but it was not found in the pinned toolchain"
                    )
                })?;
            files.push((name.clone(), fs::read(path)?));
        }
    }
    for (name, path) in [
        ("README.md", "README.md"),
        ("THIRD_PARTY_NOTICES.md", "THIRD_PARTY_NOTICES.md"),
        ("licenses/Inter-OFL.txt", "assets/fonts/Inter-OFL.txt"),
        (
            "licenses/IBMPlexMono-OFL.txt",
            "assets/fonts/IBMPlexMono-OFL.txt",
        ),
        ("examples/welcome.md", "examples/welcome.md"),
        ("examples/system.mmd", "examples/system.mmd"),
    ] {
        files.push((name.to_owned(), fs::read(root.join(path))?));
    }
    files.push((
        "licenses/dependencies.json".to_owned(),
        license_inventory(root, host, cargo, native_path)?,
    ));
    files.push((
        "native-imports.txt".to_owned(),
        imports
            .iter()
            .map(|value| format!("{value}\n"))
            .collect::<String>()
            .into_bytes(),
    ));
    let artifacts: Vec<_> = files.iter().map(|(path, bytes)| serde_json::json!({"path":path,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes))})).collect();
    files.push(("build-manifest.json".to_owned(), serde_json::to_vec_pretty(&serde_json::json!({
        "version":"0.3.0", "target":host, "language":"Rust", "ui":"native egui", "renderer":"in-house diagram engine",
        "validation":"Compilation and packaging only.",
        "signed":false, "artifacts":artifacts,
    }))?));
    for (name, bytes) in &files {
        let destination = directory.join(name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(destination, bytes)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            directory.join(executable_name),
            fs::Permissions::from_mode(0o755),
        )?;
    }
    if !windows {
        // Unix archives keep the executable bit, which the STORE zip writer cannot.
        for (name, source) in [
            ("tidemap.desktop", "packaging/linux/tidemap.desktop"),
            ("tidemap.png", "assets/icon.png"),
        ] {
            if host.contains("linux") {
                fs::copy(root.join(source), directory.join(name))?;
            }
        }
        let tarball = root.join("release").join(format!("{folder_name}.tar.gz"));
        let status = Command::new("tar")
            .current_dir(root.join("release"))
            .arg("-czf")
            .arg(&tarball)
            .arg(&folder_name)
            .status()?;
        if !status.success() {
            return Err("tar failed".into());
        }
        fs::write(
            root.join("release").join(format!("{folder_name}.tar.gz.sha256")),
            format!(
                "{:x}  {folder_name}.tar.gz
",
                Sha256::digest(fs::read(&tarball)?)
            ),
        )?;
        println!("Portable archive: {}", tarball.display());
        return Ok(());
    }
    let zip_path = root.join("release").join(format!("{folder_name}.zip"));
    let archive_files: Vec<_> = files
        .into_iter()
        .map(|(name, bytes)| (format!("{folder_name}/{name}"), bytes))
        .collect();
    write_zip(&zip_path, &archive_files)?;
    let zip = fs::read(&zip_path)?;
    fs::write(
        zip_path.with_extension("zip.sha256"),
        format!(
            "{:x}  {}\n",
            Sha256::digest(&zip),
            zip_path.file_name().unwrap().to_string_lossy()
        ),
    )?;
    println!(
        "Packaged native application: {}\nPortable archive: {} ({} bytes)",
        directory.join(executable_name).display(),
        zip_path.display(),
        zip.len()
    );
    Ok(())
}
fn license_inventory(
    root: &Path,
    host: &str,
    cargo: &Path,
    native_path: &OsString,
) -> Result<Vec<u8>> {
    let metadata = output(
        Command::new(cargo)
            .current_dir(root)
            .env("PATH", native_path)
            .args([
                "metadata",
                "--locked",
                "--format-version",
                "1",
                "--filter-platform",
                host,
            ]),
    )?;
    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("Missing Cargo packages")?;
    let app = packages
        .iter()
        .find(|package| package["name"] == "tidemap")
        .and_then(|package| package["id"].as_str())
        .ok_or("Missing native application package")?;
    let nodes: BTreeMap<_, _> = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("Missing Cargo dependency graph")?
        .iter()
        .filter_map(|node| node["id"].as_str().map(|id| (id, node)))
        .collect();
    let mut needed = BTreeSet::new();
    let mut pending = vec![app];
    while let Some(id) = pending.pop() {
        if !needed.insert(id) {
            continue;
        }
        if let Some(node) = nodes.get(id)
            && let Some(dependencies) = node["deps"].as_array()
        {
            for dependency in dependencies {
                if let Some(id) = dependency["pkg"].as_str() {
                    pending.push(id);
                }
            }
        }
    }
    let mut inventory = Vec::new();
    for package in packages
        .iter()
        .filter(|package| package["id"].as_str().is_some_and(|id| needed.contains(id)))
    {
        let manifest = package["manifest_path"]
            .as_str()
            .ok_or("Missing crate manifest")?;
        let directory = Path::new(manifest)
            .parent()
            .ok_or("Invalid crate manifest")?;
        let mut notices = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let upper = name.to_uppercase();
            if entry.file_type()?.is_file()
                && ["LICENSE", "LICENCE", "COPYING", "NOTICE"]
                    .iter()
                    .any(|prefix| upper.starts_with(prefix))
                && let Ok(text) = fs::read_to_string(entry.path())
            {
                notices.push(serde_json::json!({"file":name,"text":text}));
            }
        }
        inventory.push(serde_json::json!({"name":package["name"],"version":package["version"],"license":package["license"],"repository":package["repository"],"notices":notices}));
    }
    Ok(serde_json::to_vec_pretty(&inventory)?)
}

// Read the native executable's import table without executing it.
fn pe_imports(bytes: &[u8]) -> Result<BTreeSet<String>> {
    fn word(bytes: &[u8], offset: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(
            bytes
                .get(offset..offset + 2)
                .ok_or("Truncated PE")?
                .try_into()?,
        ))
    }
    fn dword(bytes: &[u8], offset: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("Truncated PE")?
                .try_into()?,
        ))
    }
    let pe = dword(bytes, 0x3c)? as usize;
    if bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("The native binary is not a PE executable".into());
    }
    let count = word(bytes, pe + 6)? as usize;
    let size = word(bytes, pe + 20)? as usize;
    let optional = pe + 24;
    let data = optional
        + if word(bytes, optional)? == 0x20b {
            112
        } else {
            96
        };
    let imports = dword(bytes, data + 8)?;
    let sections = optional + size;
    let address = |rva: u32| -> Result<usize> {
        for index in 0..count {
            let section = sections + index * 40;
            let start = dword(bytes, section + 12)?;
            let span = dword(bytes, section + 8)?.max(dword(bytes, section + 16)?);
            if rva >= start && rva < start.saturating_add(span) {
                return Ok((rva - start + dword(bytes, section + 20)?) as usize);
            }
        }
        Err("Unmapped PE import address".into())
    };
    let mut result = BTreeSet::new();
    if imports == 0 {
        return Ok(result);
    }
    let mut descriptor = address(imports)?;
    for _ in 0..512 {
        let name = dword(bytes, descriptor + 12)?;
        if name == 0 {
            break;
        }
        let start = address(name)?;
        let tail = bytes.get(start..).ok_or("Invalid PE name")?;
        let end = tail
            .iter()
            .take(512)
            .position(|byte| *byte == 0)
            .ok_or("Invalid PE library name")?;
        result.insert(String::from_utf8(tail[..end].to_vec())?);
        descriptor += 20;
    }
    Ok(result)
}

// Deterministic ZIP STORE writer keeps distribution tooling dependency-free.
fn write_zip(path: &Path, files: &[(String, Vec<u8>)]) -> Result<()> {
    fn u16le(out: &mut impl Write, value: u16) -> io::Result<()> {
        out.write_all(&value.to_le_bytes())
    }
    fn u32le(out: &mut impl Write, value: u32) -> io::Result<()> {
        out.write_all(&value.to_le_bytes())
    }
    fn crc(bytes: &[u8]) -> u32 {
        let mut value = !0u32;
        for byte in bytes {
            value ^= *byte as u32;
            for _ in 0..8 {
                value = (value >> 1) ^ (0xedb88320u32.wrapping_mul(value & 1));
            }
        }
        !value
    }
    let mut file = fs::File::create(path)?;
    let mut offset = 0u32;
    let mut central = Vec::new();
    for (name, bytes) in files {
        let size = u32::try_from(bytes.len())?;
        let name_len = u16::try_from(name.len())?;
        let checksum = crc(bytes);
        let local = offset;
        u32le(&mut file, 0x04034b50)?;
        u16le(&mut file, 20)?;
        u16le(&mut file, 0x800)?;
        u16le(&mut file, 0)?;
        u16le(&mut file, 0)?;
        u16le(&mut file, 33)?;
        u32le(&mut file, checksum)?;
        u32le(&mut file, size)?;
        u32le(&mut file, size)?;
        u16le(&mut file, name_len)?;
        u16le(&mut file, 0)?;
        file.write_all(name.as_bytes())?;
        file.write_all(bytes)?;
        offset = offset
            .checked_add(30 + name_len as u32 + size)
            .ok_or("ZIP exceeds native distribution bounds")?;
        u32le(&mut central, 0x02014b50)?;
        u16le(&mut central, 20)?;
        u16le(&mut central, 20)?;
        u16le(&mut central, 0x800)?;
        u16le(&mut central, 0)?;
        u16le(&mut central, 0)?;
        u16le(&mut central, 33)?;
        u32le(&mut central, checksum)?;
        u32le(&mut central, size)?;
        u32le(&mut central, size)?;
        u16le(&mut central, name_len)?;
        u16le(&mut central, 0)?;
        u16le(&mut central, 0)?;
        u16le(&mut central, 0)?;
        u16le(&mut central, 0)?;
        u32le(&mut central, 0)?;
        u32le(&mut central, local)?;
        central.write_all(name.as_bytes())?;
    }
    file.write_all(&central)?;
    u32le(&mut file, 0x06054b50)?;
    u16le(&mut file, 0)?;
    u16le(&mut file, 0)?;
    u16le(&mut file, u16::try_from(files.len())?)?;
    u16le(&mut file, u16::try_from(files.len())?)?;
    u32le(&mut file, u32::try_from(central.len())?)?;
    u32le(&mut file, offset)?;
    u16le(&mut file, 0)?;
    file.sync_all()?;
    Ok(())
}
