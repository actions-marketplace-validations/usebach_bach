//! Programs the app asks setup to run: `[[run]]` in `bach.toml`.
//!
//! This is for what copying files can't do (a runtime's own installer, a
//! driver, registering with a service), and it is deliberately narrow:
//!
//! - The program is one of the app's packaged files, found inside the install
//!   folder through the same path rules as everything else; setup never runs
//!   something from anywhere else, and never through a shell.
//! - It runs as the user, like setup. A program that asks for administrator
//!   rights fails to start, and that is reported like any other failure,
//!   unless the step says `elevated = true`: then Windows asks once and the
//!   program runs with administrator rights (setup itself stays as it is).
//! - Setup waits for it, up to [`LIMIT`], and ends it if it takes longer.
//! - A step that fails stops an install or update with a readable sentence
//!   (unless it says `ignore_errors`); a step that runs before removal never
//!   stops the removal, because an app must always be removable.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bach_pack::i18n::Key;
use bach_pack::manifest::{Choices, FileEntry, Manifest, RunStep, RunWhen};
use bach_pack::paths;

use super::Report;
use crate::system::elevate::{self, Outcome};
use crate::texts::Texts;

/// How long one step may take before setup gives up on it, unless it sets
/// `timeout_minutes`.
pub const LIMIT: Duration = Duration::from_secs(15 * 60);

/// The steps that apply to an install or update with these choices.
pub fn due_for_install<'a>(manifest: &'a Manifest, first_install: bool, choices: &Choices) -> Vec<&'a RunStep> {
    manifest
        .run
        .iter()
        .filter(|step| match step.when {
            RunWhen::Install => true,
            RunWhen::FirstInstall => first_install,
            RunWhen::Uninstall => false,
        })
        .filter(|step| step.component.as_ref().is_none_or(|component| choices.components.contains(component)))
        .collect()
}

/// The steps to run before removing: those whose file was installed.
pub fn due_for_removal(manifest: &Manifest) -> Vec<&RunStep> {
    manifest
        .run
        .iter()
        .filter(|step| step.when == RunWhen::Uninstall)
        .filter(|step| manifest.files.iter().any(|file| file.path.eq_ignore_ascii_case(&step.file)))
        .collect()
}

/// Runs the install steps, `from`..`to` along the progress bar. The first
/// failure that isn't ignored ends it.
pub fn run_install_steps(
    manifest: &Manifest,
    dir: &Path,
    first_install: bool,
    choices: &Choices,
    texts: &Texts,
    (from, to): (f32, f32),
    report: Report,
) -> Result<(), String> {
    let steps = due_for_install(manifest, first_install, choices);
    for (index, step) in steps.iter().enumerate() {
        report(from + (to - from) * index as f32 / steps.len() as f32, &texts.with(Key::step_running, &[("task", &step.name)]));
        if let Err(message) = run(step, manifest_file(manifest, step), dir, texts) {
            crate::log::line(&message);
            if !step.ignore_errors {
                return Err(message);
            }
            crate::log::line("Going on, as the step says to.");
        }
    }
    Ok(())
}

/// Runs the steps that come before removal. Whatever they do, removal goes on.
pub fn run_removal_steps(manifest: &Manifest, dir: &Path, texts: &Texts, report: Report, at: f32) {
    for step in due_for_removal(manifest) {
        report(at, &texts.with(Key::step_running, &[("task", &step.name)]));
        if let Err(message) = run(step, manifest_file(manifest, step), dir, texts) {
            crate::log::line(&format!("{message} Removing goes on anyway."));
        }
    }
}

/// How a program setup started ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    /// It ran and ended with this exit code.
    Code(i32),
    /// The user said no to Windows' administrator prompt.
    Declined,
    /// It ran longer than [`LIMIT`], so setup ended it.
    TimedOut,
    /// It never started; this is Windows' error number, or -1.
    NotStarted(String),
}

/// Starts `program` in its own folder and waits for it, up to `limit`. With
/// `elevated`, Windows asks for administrator rights first (and doesn't when
/// setup has them already). The arguments are passed as written, and never
/// through a shell.
///
/// What the program prints goes to the log (its last [`OUTPUT_KEPT`] bytes, so
/// "exit code 1603" comes with the reason), and the programs it starts belong
/// to a job that ends with it, so a timeout leaves nothing behind holding files.
pub fn execute(program: &Path, arguments: Option<&str>, elevated: bool, limit: Duration) -> Ended {
    let arguments_text = arguments.unwrap_or("");
    let cwd = program.parent();
    if elevated && !elevate::is_elevated() {
        crate::log::line(&format!("Running {} {arguments_text} with administrator rights", program.display()));
        return match elevate::run(program, arguments_text, cwd, limit, true) {
            Outcome::Exited(code) => {
                // Exit codes are signed on the command line: 0xC0000005 reads as -1073741819.
                let code = code as i32;
                crate::log::line(&format!("It ended with code {code}."));
                Ended::Code(code)
            }
            Outcome::Declined => {
                crate::log::line("Windows' administrator prompt was declined.");
                Ended::Declined
            }
            Outcome::TimedOut => {
                crate::log::line("It took too long, so setup stopped waiting for it.");
                Ended::TimedOut
            }
            Outcome::Failed(why) => {
                crate::log::line(&format!("{} didn't start: {why}", program.display()));
                Ended::NotStarted("-1".into())
            }
        };
    }
    let mut command = Command::new(program);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if let Some(arguments) = arguments {
        // The arguments are the developer's command line, passed as written.
        command.raw_arg(arguments);
    }
    crate::log::line(&format!("Running {} {arguments_text}", program.display()));
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            crate::log::line(&format!("{} didn't start: {err}", program.display()));
            return Ended::NotStarted(err.raw_os_error().map_or_else(|| "-1".to_string(), |code| code.to_string()));
        }
    };
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    for pipe in [child.stdout.take().map(Pipe::Out), child.stderr.take().map(Pipe::Err)].into_iter().flatten() {
        let output = output.clone();
        std::thread::spawn(move || pipe.drain_into(&output));
    }
    // Dropping the job ends everything still in it, whichever way this returns.
    let _job = tree::Job::containing(&child);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                log_output(&output);
                crate::log::line("It took too long, so setup ended it, and everything it had started.");
                return Ended::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(150)),
            Err(err) => {
                crate::log::line(&format!("Couldn't wait for it: {err}"));
                return Ended::NotStarted("-1".into());
            }
        }
    };
    // The pipes close when the program does; give the readers a moment.
    std::thread::sleep(Duration::from_millis(100));
    log_output(&output);
    let code = status.code().unwrap_or(-1);
    crate::log::line(&format!("It ended with code {code}."));
    Ended::Code(code)
}

/// How much of a program's output the log keeps: the end, which is where the
/// reason for a failure is.
pub const OUTPUT_KEPT: usize = 8 * 1024;

/// One of a program's two output streams.
enum Pipe {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

impl Pipe {
    /// Reads until the stream closes, keeping only the last [`OUTPUT_KEPT`] bytes.
    fn drain_into(self, kept: &Mutex<Vec<u8>>) {
        use std::io::Read;
        let mut reader: Box<dyn Read> = match self {
            Pipe::Out(pipe) => Box::new(pipe),
            Pipe::Err(pipe) => Box::new(pipe),
        };
        let mut buffer = [0u8; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            if let Ok(mut kept) = kept.lock() {
                kept.extend_from_slice(&buffer[..count]);
                if kept.len() > OUTPUT_KEPT {
                    let excess = kept.len() - OUTPUT_KEPT;
                    kept.drain(..excess);
                }
            }
        }
    }
}

/// Writes what the program printed to the log, line by line.
fn log_output(output: &Mutex<Vec<u8>>) {
    let Ok(bytes) = output.lock().map(|bytes| bytes.clone()) else {
        return;
    };
    // Programs print in the console's code page or UTF-16; lossy is enough to read a reason.
    for line in String::from_utf8_lossy(&bytes).replace('\0', "").lines().map(str::trim_end).filter(|line| !line.is_empty()) {
        crate::log::line(&format!("  | {line}"));
    }
}

/// A Windows job object that ends its processes when it is closed.
mod tree {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Job(HANDLE);

    impl Job {
        /// A job holding `child` (and so whatever it starts), or None when
        /// Windows won't make one; the step then runs as it always did.
        pub fn containing(child: &Child) -> Option<Job> {
            // SAFETY: plain handle calls; the job handle is closed in Drop and the
            // limit structure is fully initialised before it is passed.
            unsafe {
                let job = CreateJobObjectW(None, None).ok()?;
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                let assigned = set.is_ok() && AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())).is_ok();
                if assigned {
                    Some(Job(job))
                } else {
                    let _ = CloseHandle(job);
                    None
                }
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateJobObjectW and is closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// What the manifest records about the step's program, when it is a packaged file.
fn manifest_file<'a>(manifest: &'a Manifest, step: &RunStep) -> Option<&'a FileEntry> {
    manifest.files.iter().find(|file| paths::fold_case(&file.path) == paths::fold_case(&step.file))
}

/// Starts the step's program and waits for it. `recorded` is what the manifest says the
/// file is: an elevated step starts a program from a folder the user's own programs can
/// write to (a per-user install), so right before Windows is asked to give it administrator
/// rights the file is checked to be what setup wrote, and a program that was swapped in the
/// meantime is not started.
pub fn run(step: &RunStep, recorded: Option<&FileEntry>, dir: &Path, texts: &Texts) -> Result<(), String> {
    let failed = |code: String| texts.with(Key::error_run, &[("task", &step.name), ("code", &code)]);
    let program = paths::under(dir, &step.file);
    if step.elevated && !elevate::is_elevated() {
        if let Some(file) = recorded.filter(|file| file.sha256.is_some()) {
            if !super::patch::matches_on_disk(&program, file) {
                crate::log::line(&format!(
                    "{} isn't what setup wrote (its size or SHA-256 differs from the manifest's), so it isn't started with administrator rights.",
                    program.display()
                ));
                return Err(failed("integrity".into()));
            }
        }
    }
    match execute(&program, step.arguments.as_deref(), step.elevated, step.timeout()) {
        Ended::Code(code) if step.succeeded(code) => Ok(()),
        Ended::Code(code) => Err(failed(code.to_string())),
        // 1223 is ERROR_CANCELLED, what Windows itself reports for it.
        Ended::Declined => Err(failed("1223".into())),
        Ended::TimedOut => Err(failed("timeout".into())),
        Ended::NotStarted(code) => Err(failed(code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;
    use bach_pack::manifest::FileEntry;

    fn texts() -> Texts {
        let manifest = Package::sample().manifest;
        Texts::in_language(&manifest, &bach_pack::i18n::LANGUAGES[0])
    }

    /// A folder with a copy of `cmd.exe` named `tool.exe` in it.
    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bach-steps-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let cmd = Path::new(&std::env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe");
        std::fs::copy(cmd, dir.join("bin/tool.exe")).unwrap();
        dir
    }

    fn step(arguments: &str) -> RunStep {
        RunStep {
            name: "Helper".into(),
            file: "bin/tool.exe".into(),
            arguments: Some(arguments.into()),
            when: RunWhen::Install,
            success_codes: vec![0],
            ignore_errors: false,
            component: None,
            elevated: false,
            timeout_minutes: None,
        }
    }

    #[test]
    fn only_the_end_of_a_long_output_is_kept() {
        let mut child = Command::new("cmd").args(["/c", "for /L %i in (1,1,3000) do @echo line%i"]).stdout(Stdio::piped()).spawn().unwrap();
        let kept = Mutex::new(Vec::new());
        Pipe::Out(child.stdout.take().unwrap()).drain_into(&kept);
        let _ = child.wait();
        let text = String::from_utf8_lossy(&kept.lock().unwrap()).to_string();
        assert!(kept.lock().unwrap().len() <= OUTPUT_KEPT);
        assert!(text.contains("line3000") && !text.contains("line1\r\n"), "the end stays, the start goes");
    }

    #[test]
    fn a_timeout_ends_the_programs_the_step_started_too() {
        let dir = folder("tree");
        let cmd = Path::new(&std::env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe");
        std::fs::copy(cmd, dir.join("bin/bachqagrandchild.exe")).unwrap();
        // tool.exe (a cmd) starts a second, uniquely named cmd, which waits a minute.
        let ended = execute(
            &dir.join("bin/tool.exe"),
            Some(r"/c .\bachqagrandchild.exe /c waitfor /t 60 bachqasignal >nul 2>&1"),
            false,
            Duration::from_secs(2),
        );
        assert!(matches!(ended, Ended::TimedOut));
        std::thread::sleep(Duration::from_millis(700));
        let listing = Command::new("tasklist").args(["/FI", "IMAGENAME eq bachqagrandchild.exe"]).output().unwrap();
        assert!(!String::from_utf8_lossy(&listing.stdout).to_lowercase().contains("bachqagrandchild"), "the grandchild must be gone");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_step_that_succeeds_passes() {
        let dir = folder("ok");
        assert!(run(&step("/c exit 0"), None, &dir, &texts()).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failing_step_says_which_and_with_what_code() {
        let dir = folder("fail");
        let message = run(&step("/c exit 7"), None, &dir, &texts()).unwrap_err();
        assert!(message.contains("Helper") && message.contains('7'), "{message}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn listed_exit_codes_count_as_success() {
        let dir = folder("codes");
        let mut step = step("/c exit 3010");
        assert!(run(&step, None, &dir, &texts()).is_err());
        step.success_codes = vec![0, 3010];
        assert!(run(&step, None, &dir, &texts()).is_ok(), "3010 is \"restart needed\", which many installers use");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn arguments_reach_the_program_as_written() {
        let dir = folder("args");
        // cmd's own quoting rules show the quotes arrived intact.
        assert!(run(&step("/c \"if \"a\"==\"a\" (exit 0) else (exit 9)\""), None, &dir, &texts()).is_ok());
        assert!(run(&step("/c \"if \"a\"==\"b\" (exit 0) else (exit 9)\""), None, &dir, &texts()).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_program_that_is_not_there_fails_to_start() {
        let dir = folder("missing");
        let mut missing = step("");
        missing.file = "bin/not-there.exe".into();
        let message = run(&missing, None, &dir, &texts()).unwrap_err();
        assert!(message.contains("Helper"), "{message}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn it_runs_in_its_own_folder() {
        let dir = folder("cwd");
        // The marker file only exists next to the program.
        std::fs::write(dir.join("bin/marker.txt"), "x").unwrap();
        assert!(run(&step("/c if exist marker.txt (exit 0) else (exit 5)"), None, &dir, &texts()).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn manifest_with(steps: Vec<RunStep>) -> Manifest {
        let mut manifest = Package::sample().manifest;
        manifest.files = vec![
            FileEntry::new("sample.exe", 1),
            FileEntry::new("bin/tool.exe", 1),
            FileEntry { path: "extras/more.exe".into(), size: 1, component: Some("extras".into()), sha256: None },
        ];
        manifest.run = steps;
        manifest
    }

    #[test]
    fn the_right_steps_are_due() {
        let mut first = step("");
        first.when = RunWhen::FirstInstall;
        let always = step("");
        let mut after = step("");
        after.when = RunWhen::Uninstall;
        let mut optional = step("");
        optional.component = Some("extras".into());
        let manifest = manifest_with(vec![first, always, after, optional]);
        let none = Choices::default();
        let with_extras = Choices { components: vec!["extras".into()], ..Choices::default() };

        assert_eq!(due_for_install(&manifest, true, &none).len(), 2, "first install: first_install + install");
        assert_eq!(due_for_install(&manifest, false, &none).len(), 1, "an update skips first_install");
        assert_eq!(due_for_install(&manifest, true, &with_extras).len(), 3, "the optional one joins when ticked");
        assert_eq!(due_for_removal(&manifest).len(), 1);
    }

    #[test]
    fn removal_only_runs_steps_whose_file_was_installed() {
        let mut gone = step("");
        gone.when = RunWhen::Uninstall;
        gone.file = "extras/not-installed.exe".into();
        let mut here = step("");
        here.when = RunWhen::Uninstall;
        let manifest = manifest_with(vec![gone, here]);
        assert_eq!(due_for_removal(&manifest).len(), 1);
    }

    #[test]
    fn removal_goes_on_when_a_step_fails() {
        let dir = folder("removal");
        let mut failing = step("/c exit 4");
        failing.when = RunWhen::Uninstall;
        let manifest = manifest_with(vec![failing]);
        let seen = std::cell::RefCell::new(Vec::new());
        // Returns instead of erroring: that is the whole contract.
        run_removal_steps(&manifest, &dir, &texts(), &|_, label| seen.borrow_mut().push(label.to_string()), 0.1);
        assert_eq!(seen.borrow().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn install_steps_stop_at_a_failure_unless_told_to_go_on() {
        let dir = folder("install");
        let mut failing = step("/c exit 4");
        let manifest = manifest_with(vec![failing.clone(), step("/c exit 0")]);
        let choices = Choices::default();
        let result = run_install_steps(&manifest, &dir, true, &choices, &texts(), (0.9, 0.95), &|_, _| {});
        assert!(result.is_err());

        failing.ignore_errors = true;
        let manifest = manifest_with(vec![failing, step("/c exit 0")]);
        assert!(run_install_steps(&manifest, &dir, true, &choices, &texts(), (0.9, 0.95), &|_, _| {}).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_elevated_program_that_was_swapped_is_not_started() {
        // Elevation is only asked for when setup has none; a test run from an elevated shell
        // takes the ordinary path, where the check is not needed.
        if elevate::is_elevated() {
            return;
        }
        let dir = folder("swapped");
        let program = dir.join("bin/tool.exe");
        let mut recorded =
            FileEntry { path: "bin/tool.exe".into(), size: std::fs::metadata(&program).unwrap().len(), sha256: None, component: None };
        recorded.sha256 = Some(bach_pack::protect::sha256_hex(std::fs::File::open(&program).unwrap()).unwrap());
        let mut elevated = step("/c exit 0");
        elevated.elevated = true;
        // Swap the file for another one of a different size: the step must fail before Windows is asked.
        std::fs::write(&program, b"MZ not what setup wrote").unwrap();
        let message = run(&elevated, Some(&recorded), &dir, &texts()).unwrap_err();
        assert!(message.contains("integrity"), "{message}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
