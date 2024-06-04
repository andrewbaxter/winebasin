use {
    aargvark::{
        vark,
        Aargvark,
    },
    command_fds::{
        CommandFdExt,
        FdMapping,
    },
    defer::defer,
    directories::ProjectDirs,
    format_bytes::format_bytes,
    loga::{
        ea,
        fatal,
        DebugDisplay,
        ResultContext,
        StandardFlag,
        StandardLog,
    },
    os_pipe::pipe,
    serde::{
        Deserialize,
        Serialize,
    },
    shlex::bytes::try_quote,
    std::{
        cell::OnceCell,
        collections::HashMap,
        env::{
            self,
            current_dir,
        },
        ffi::{
            OsStr,
            OsString,
        },
        fs::{
            self,
            create_dir_all,
        },
        io::{
            BufRead,
            BufReader,
            Write,
        },
        os::{
            linux::fs::MetadataExt,
            unix::ffi::{
                OsStrExt,
                OsStringExt,
            },
        },
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Stdio,
        },
        rc::Rc,
    },
};

#[derive(Aargvark, Serialize, Deserialize, Clone, Copy)]
enum Arch {
    Win32,
    Win64,
}

#[derive(Serialize, Deserialize, Clone)]
struct BasisConfigV1 {
    arch: Arch,
}

type BasisLatestConfig = BasisConfigV1;

#[derive(Serialize, Deserialize)]
enum BasisConfig {
    V1(BasisConfigV1),
}

#[derive(Serialize, Deserialize)]
struct SystemConfigV1 {
    basis_name: String,
}

type SystemLatestConfig = SystemConfigV1;

#[derive(Serialize, Deserialize)]
enum SystemConfig {
    V1(SystemConfigV1),
}

#[derive(Aargvark)]
struct BasisCreateArgs {
    basis_name: String,
    /// Defaults to `win64`.
    arch: Option<Arch>,
    /// Install a recommended 5Gb of winetricks dlls to the prefix
    recommended_winetricks: Option<()>,
}

#[derive(Aargvark)]
struct BasisShellArgs {
    basis_name: String,
    /// Command to run in the shell, such as a script. If empty, interactive shell.
    command: Vec<String>,
}

#[derive(Aargvark)]
#[vark(break)]
enum BasisArgs {
    /// Create a new basis
    Create(BasisCreateArgs),
    /// Confirm a basis can be used with the current wine version without update. Exits
    /// with 1 if update needed.
    Check {
        basis_name: String,
    },
    /// Update the basis if required to run programs in the current wine version.
    Update {
        basis_name: String,
    },
    /// Open a shell inside the basis `drive_c` dir
    Shell(BasisShellArgs),
    /// Print the path to the basis directory (contains basis prefix and other config).
    Path {
        basis_name: String,
    },
}

#[derive(Aargvark)]
struct SystemShellArgs {
    system_name: String,
    /// Command to run in the shell, such as a script. If empty, interactive shell.
    command: Vec<String>,
}

#[derive(Aargvark)]
struct SystemRunArgs {
    system_name: String,
    /// Command and arguments, relative to `drive_c`.
    command: Vec<String>,
    /// Working directory of command - defaults to parent directory of specified
    /// command.
    working_dir: Option<PathBuf>,
}

#[derive(Aargvark)]
#[vark(break)]
enum SystemArgs {
    /// Create a new system using the specified basis.
    Create {
        basis_name: String,
        system_name: String,
    },
    /// Start a system (mount merged prefix) and open a shell inside the system's
    /// `drive_c` dir. The system prefix will be unmounted when the shell exits.
    /// Creates the system if it doesn't already exist.
    Shell(SystemShellArgs),
    /// Start a system (mount merged prefix) and run a program inside the system's
    /// `drive_c` dir. Creates the system if it doesn't already exist.
    Run(SystemRunArgs),
    /// Print the path to the system directory (prefix layer containing files not in
    /// the basis). Creates the system if it doesn't already exist.
    Path {
        system_name: String,
    },
}

#[derive(Aargvark)]
#[vark(break)]
enum Args {
    System(SystemArgs),
    Basis(BasisArgs),
}

trait ToOsString {
    fn to_os_str(&self) -> OsString;
}

impl ToOsString for [u8] {
    fn to_os_str(&self) -> OsString {
        return OsStr::from_bytes(self).to_os_string();
    }
}

impl ToOsString for str {
    fn to_os_str(&self) -> OsString {
        return self.as_bytes().to_os_str();
    }
}

impl ToOsString for String {
    fn to_os_str(&self) -> OsString {
        return self.as_bytes().to_os_str();
    }
}

trait CommandRun {
    fn run(&mut self) -> Result<(), loga::Error>;
    fn run_stdin(&mut self, stdin: &[u8]) -> Result<(), loga::Error>;
}

impl CommandRun for Command {
    fn run(&mut self) -> Result<(), loga::Error> {
        let log = StandardLog::new().fork(ea!(command = self.dbg_str()));
        let res =
            self
                .spawn()
                .stack_context(&log, "Failed to spawn command")?
                .wait()
                .stack_context(&log, "Error running command")?;
        if !res.success() {
            return Err(log.err_with("Command exited with error", ea!(status = res)));
        }
        return Ok(());
    }

    fn run_stdin(&mut self, stdin: &[u8]) -> Result<(), loga::Error> {
        let log = StandardLog::new().fork(ea!(command = self.dbg_str()));
        self.stdin(Stdio::piped());
        let mut child = self.spawn().stack_context(&log, "Error starting shell")?;
        let mut child_stdin = child.stdin.take().unwrap();
        child_stdin.write_all(stdin).stack_context(&log, "Error sending script to shell")?;
        drop(child_stdin);
        let res = child.wait().stack_context(&log, "Error waiting for shell to exit")?;
        if !res.success() {
            return Err(log.err_with("Command exited with unsuccessful code", ea!(status = res)));
        }
        return Ok(());
    }
}

fn quote_subcommand<'a>(subcommand: impl IntoIterator<Item = &'a [u8]>) -> Result<Vec<u8>, loga::Error> {
    let mut out: Vec<u8> = vec![];
    for (i, arg) in subcommand.into_iter().enumerate() {
        if i > 0 {
            out.extend(b" ");
        }
        out.extend(
            try_quote(arg)
                .context_with(
                    format!("Error escaping argument {}", i),
                    ea!(previous_args = String::from_utf8_lossy(&out), argument = String::from_utf8_lossy(arg)),
                )?
                .to_vec(),
        );
    }
    return Ok(out);
}

#[allow(dyn_drop)]
fn mount_prefix(
    log: &StandardLog,
    basis_path: &Path,
    system_path: &Path,
) -> Result<(Box<dyn Drop>, PathBuf), loga::Error> {
    let root_dir = root_dir()?;
    let tempdirs_path = root_dir.join("temp");
    create_dir_all(
        &tempdirs_path,
    ).context_with("Error creating temp dir", ea!(path = tempdirs_path.to_string_lossy()))?;

    // Launch background sudo process (keep it open so don't need reauth at exit)
    let (sudo_read, sudo_read_child) = pipe().context("Error creating sudo read pipe pair")?;
    let mut sudo_read = BufReader::new(sudo_read).lines();
    let mut sudo =
        Command::new("sudo")
            .arg("--close-from")
            .arg("4")
            .arg("bash")
            .arg("-eu")
            .stdin(Stdio::piped())
            .fd_mappings(vec![FdMapping {
                parent_fd: sudo_read_child.into(),
                child_fd: 3,
            }])
            .context("Error attaching pipes to sudo child")?
            .spawn()
            .context("Error starting cleanup bash process")?;
    let mut sudo_write = sudo.stdin.take().unwrap();
    let mut sudo_exec = {
        let mut i = 0;
        move |line: &[u8]| {
            sudo_write.write_all(line)?;
            sudo_write.write_all(b";\n")?;
            let want_i = i.to_string();
            i += 1;
            sudo_write.write_all(&format_bytes!(b"echo {} >&3;\n", want_i.as_bytes()))?;
            sudo_write.flush()?;
            while let Some(line) = sudo_read.next() {
                let line = line.context("Error reading ipc line")?;
                let line = line.trim();
                if line == want_i {
                    break;
                }
            }
            return Ok(()) as Result<_, loga::Error>;
        }
    };

    // Mount
    let mount_path = system_mount_path(&system_path);
    sudo_exec(
        &quote_subcommand(
            [
                b"mount" as &[u8],
                b"--types",
                b"overlay",
                b"overlay",
                b"--options",
                &format_bytes!(
                    b"lowerdir={},upperdir={},workdir={},metacopy=off,index=off",
                    basis_prefix_path(&basis_path).as_os_str().as_bytes(),
                    system_prefix_path(&system_path).as_os_str().as_bytes(),
                    system_overlay_work_path(&system_path).as_os_str().as_bytes()
                ),
                mount_path.as_os_str().as_bytes(),
            ],
        )?,
    )?;
    return Ok((
        // Unmount when dropped
        Box::new(defer({
            let log = log.clone();
            let mount_path = mount_path.clone();
            move || {
                (|| {
                    sudo_exec(&quote_subcommand([b"umount", mount_path.as_os_str().as_bytes()]).unwrap())?;
                    drop(sudo_exec);
                    let res = sudo.wait_with_output()?;
                    if !res.status.success() {
                        log.log_with(
                            StandardFlag::Warning,
                            "Cleanup sudo process exited with error",
                            ea!(output = res.dbg_str()),
                        );
                    }
                    return Ok(()) as Result<_, loga::Error>;
                })().log(&log, StandardFlag::Warning, "Error completing cleanup");
            }
        })),
        // Useful return
        mount_path,
    ));
}

fn shell_commandline(basis_config: &BasisLatestConfig, prefix_path: &Path) -> Command {
    let mut commandline =
        Command::new(&PathBuf::from(env::var("SHELL").as_ref().map(|x| x.as_str()).unwrap_or("/bin/bash")));
    commandline.envs(wine_envs(&basis_config, &prefix_path)).current_dir(&prefix_path.join("drive_c"));
    return commandline;
}

fn run_shell(basis_config: &BasisLatestConfig, prefix_path: &Path, command: Vec<String>) -> Result<(), loga::Error> {
    let mut commandline = shell_commandline(basis_config, prefix_path);
    if command.is_empty() {
        commandline.run()?;
    } else {
        let cwd = current_dir().context("Can't determine current dir")?;
        let command = command.into_iter().map(|x| {
            if x.starts_with("./") {
                cwd.join(x).into_os_string().into_vec()
            } else {
                x.into_bytes()
            }
        }).collect::<Vec<Vec<u8>>>();
        commandline.run_stdin(&quote_subcommand(command.iter().map(|x| x.as_ref()))?)?;
    }
    return Ok(());
}

fn root_dir() -> Result<Rc<PathBuf>, loga::Error> {
    static mut PROJECT_DIRS: OnceCell<Result<Rc<PathBuf>, loga::Error>> = OnceCell::new();
    return unsafe {
        PROJECT_DIRS.get_or_init(
            || ProjectDirs::from("", "", "winebasin")
                .context("Could not determine system directories")
                .map(|x| Rc::new(x.data_dir().to_path_buf())),
        )
    }.clone();
}

fn basis_path(name: &str) -> Result<PathBuf, loga::Error> {
    return Ok(root_dir()?.join("basis").join(name));
}

fn basis_config_path(basis_path: &Path) -> PathBuf {
    return basis_path.join("config.json");
}

fn basis_prefix_path(basis_path: &Path) -> PathBuf {
    return basis_path.join("prefix");
}

fn basis_needs_update(basis_path: &Path) -> Result<bool, loga::Error> {
    let log = StandardLog::new().fork(ea!(path = basis_path.to_string_lossy()));
    if !basis_path.exists() {
        return Err(log.err("Basis doesn't exist"));
    }
    let have_time_string =
        String::from_utf8_lossy(
            &fs::read(
                basis_prefix_path(basis_path).join(".update-timestamp"),
            ).stack_context(&log, "Error reading prefix update timestamp file")?,
        )
            .trim()
            .to_string();
    let have_time =
        i64::from_str_radix(
            &have_time_string,
            10,
        ).stack_context_with(
            &log,
            "Error parsing timestamp in prefix update timestamp file",
            ea!(timestamp = have_time_string),
        )?;

    // From
    // https://github.com/wine-mirror/wine/blob/951e0e27a743e52c75c7fedc0b1eaa9eb77e6bb6/programs/wineboot/wineboot.c#L93
    // except... that's set by another binary, where it's built in as a compile-time
    // define. Just hard code and nix users will need to set an env var.
    let inf_path = if let Some(d) = env::var("WINE_INF_DIR").ok() {
        PathBuf::from(d)
    } else {
        PathBuf::from("/usr/share/wine/wine.inf")
    };
    let inf_meta = inf_path.metadata().context("Error getting metadata of wine.inf")?;
    return Ok(have_time < inf_meta.st_mtime());
}

fn wine_envs(config: &BasisLatestConfig, prefix: &Path) -> HashMap<&'static OsStr, OsString> {
    let mut out = HashMap::new();
    out.insert(OsStr::from_bytes("WINEPREFIX".as_bytes()), prefix.as_os_str().to_os_string());
    out.insert(OsStr::from_bytes("WINEARCH".as_bytes()), OsStr::from_bytes(match config.arch {
        Arch::Win32 => "win32",
        Arch::Win64 => "win64",
    }.as_bytes()).to_os_string());
    return out;
}

fn wine_bin() -> String {
    return env::var("WINE").ok().unwrap_or_else(|| "wine".to_string());
}

fn wine_hostname(config: &BasisLatestConfig, prefix_path: &Path) -> Result<(), loga::Error> {
    Command::new("wine").arg("hostname").envs(wine_envs(config, prefix_path)).stdout(Stdio::null()).run()?;
    return Ok(());
}

fn update_basis(basis_path: &Path) -> Result<BasisLatestConfig, loga::Error> {
    let config_path = basis_config_path(basis_path);
    let config =
        match serde_json::from_slice::<BasisConfig>(
            &fs::read(
                &config_path,
            ).context_with("Error reading basis config", ea!(path = config_path.to_string_lossy()))?,
        ).context_with("Error parsing basis config as json", ea!(path = config_path.to_string_lossy()))? {
            BasisConfig::V1(config) => config,
        };
    if !basis_needs_update(basis_path)? {
        return Ok(config);
    }
    let prefix_path = basis_prefix_path(basis_path);
    wine_hostname(&config, &prefix_path)?;
    return Ok(config);
}

fn system_path(name: &str) -> Result<PathBuf, loga::Error> {
    return Ok(root_dir()?.join("system").join(name));
}

fn system_config_path(system_path: &Path) -> PathBuf {
    return system_path.join("config.json");
}

fn system_prefix_path(system_path: &Path) -> PathBuf {
    return system_path.join("prefix");
}

fn system_overlay_work_path(system_path: &Path) -> PathBuf {
    return system_path.join("overlay_work");
}

fn system_mount_path(system_path: &Path) -> PathBuf {
    return system_path.join("mount");
}

fn check_system(system_path: &Path) -> Result<SystemLatestConfig, loga::Error> {
    let config_path = basis_config_path(system_path);
    let config =
        match serde_json::from_slice::<SystemConfig>(
            &fs::read(
                &config_path,
            ).context_with("Error reading system config", ea!(path = config_path.to_string_lossy()))?,
        ).context_with("Error parsing system config as json", ea!(path = config_path.to_string_lossy()))? {
            SystemConfig::V1(config) => config,
        };
    return Ok(config);
}

fn main() {
    match (|| {
        let args = vark::<Args>();
        let log = StandardLog::new().with_flags(&[StandardFlag::Error, StandardFlag::Warning, StandardFlag::Info]);
        match args {
            Args::Basis(args) => match args {
                BasisArgs::Create(args) => {
                    let basis_path = basis_path(&args.basis_name)?;
                    let log = log.fork(ea!(path = basis_path.to_string_lossy()));
                    if basis_path.exists() {
                        return Err(
                            log.err("Basis already exists. Delete the directory first if you want to re-create it"),
                        );
                    }
                    create_dir_all(&basis_path).context("Failed to ensure basis directory")?;
                    let arch = args.arch.unwrap_or(Arch::Win64);
                    let config = BasisLatestConfig { arch: arch };
                    let config_path = basis_config_path(&basis_path);
                    fs::write(
                        &config_path,
                        &serde_json::to_vec_pretty(&BasisConfig::V1(config.clone())).unwrap(),
                    ).stack_context_with(
                        &log,
                        "Error writing basis config",
                        ea!(config = config_path.to_string_lossy()),
                    )?;
                    let prefix_path = basis_prefix_path(&basis_path);
                    wine_hostname(&config, &prefix_path)?;
                    if args.recommended_winetricks.is_some() {
                        let mut commandline = shell_commandline(&config, &prefix_path);
                        match arch {
                            Arch::Win32 => {
                                commandline.run_stdin(include_bytes!("../winetricks32.sh"))?;
                            },
                            Arch::Win64 => {
                                commandline.run_stdin(include_bytes!("../winetricks64.sh"))?;
                            },
                        }
                    }
                },
                BasisArgs::Check { basis_name } => {
                    let basis_path = basis_path(&basis_name)?;
                    print!("{}", basis_needs_update(&basis_path)?);
                },
                BasisArgs::Update { basis_name } => {
                    let basis_path = basis_path(&basis_name)?;
                    update_basis(&basis_path)?;
                },
                BasisArgs::Shell(args) => {
                    let basis_path = basis_path(&args.basis_name)?;
                    let basis_config = update_basis(&basis_path)?;
                    let log = log.fork(ea!(path = basis_path.to_string_lossy()));
                    if !basis_path.exists() {
                        return Err(log.err("Basis doesn't exist"));
                    }
                    run_shell(&basis_config, &basis_prefix_path(&basis_path), args.command)?;
                },
                BasisArgs::Path { basis_name } => {
                    print!("{}", basis_path(&basis_name)?.to_string_lossy());
                },
            },
            Args::System(args) => match args {
                SystemArgs::Create { basis_name, system_name } => {
                    let system_path = system_path(&system_name)?;
                    create_dir_all(
                        &system_path,
                    ).context_with("Error creating system path", ea!(path = system_path.to_string_lossy()))?;
                    create_dir_all(
                        &system_prefix_path(&system_path),
                    ).context("Failed to ensure system prefix directory")?;
                    create_dir_all(
                        &system_overlay_work_path(&system_path),
                    ).context("Failed to ensure system overlay work directory")?;
                    create_dir_all(
                        &system_mount_path(&system_path),
                    ).context("Failed to ensure system overlay mount directory")?;
                    let config_path = system_config_path(&system_path);
                    fs::write(
                        &config_path,
                        serde_json::to_vec_pretty(
                            &SystemConfig::V1(SystemLatestConfig { basis_name: basis_name }),
                        ).unwrap(),
                    ).context_with("Error writing config to system dir", ea!(path = config_path.to_string_lossy()))?;
                },
                SystemArgs::Shell(args) => {
                    let system_path = system_path(&args.system_name)?;
                    let system_config = check_system(&system_path)?;
                    let basis_path = basis_path(&system_config.basis_name)?;
                    let basis_config = update_basis(&basis_path)?;
                    let (_mount, mount_path) = mount_prefix(&log, &basis_path, &system_path)?;
                    run_shell(&basis_config, &mount_path, args.command)?;
                },
                SystemArgs::Run(mut args) => {
                    let system_path = system_path(&args.system_name)?;
                    let system_config = check_system(&system_path)?;
                    let basis_path = basis_path(&system_config.basis_name)?;
                    let basis_config = update_basis(&basis_path)?;
                    let (_mount, mount_path) = mount_prefix(&log, &basis_path, &system_path)?;
                    let drive_c_path = mount_path.join("drive_c");
                    let command_args = args.command.split_off(1);
                    let command_command =
                        drive_c_path.join(args.command.pop().context("Command line to run in system is empty")?);
                    Command::new(wine_bin())
                        .envs(wine_envs(&basis_config, &mount_path))
                        .current_dir(&args.working_dir.unwrap_or(drive_c_path))
                        .arg(command_command)
                        .args(command_args)
                        .run()?;
                },
                SystemArgs::Path { system_name } => {
                    let system_path = system_path(&system_name)?;
                    print!("{}", system_path.to_string_lossy());
                },
            },
        }
        return Ok(()) as Result<_, loga::Error>;
    })() {
        Ok(_) => { },
        Err(e) => {
            fatal(e);
        },
    }
}
