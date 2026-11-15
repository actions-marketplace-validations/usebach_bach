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
use std::time::{Duration, Instant};

use bach_pack::i18n::Key;
use bach_pack::manifest::{Choices, Manifest, RunStep, RunWhen};
use bach_pack::paths;

use super::Report;
use crate::system::elevate::{self, Outcome};
use crate::texts::Texts;

/// How long one step may take before setup gives up on it.
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
        if let Err(message) = run(step, dir, texts) {
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
        if let Err(message) = run(step, dir, texts) {
            crate::log::line(&format!("{message} Removing goes on anyway."));
        }
    }
}

/// Starts the step's program and waits for it.
pub fn run(step: &RunStep, dir: &Path, texts: &Texts) -> Result<(), String> {
    let failed = |code: String| texts.with(Key::error_run, &[("task", &step.name), ("code", &code)]);
    let program = paths::under(dir, &step.file);
    // Setup already elevated (a machine-wide install) needs no second prompt.
    if step.elevated && !elevate::is_elevated() {
        return run_elevated(step, &program, &failed);
    }
    let mut command = Command::new(&program);
    command.current_dir(program.parent().unwrap_or(dir)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(arguments) = &step.arguments {
        // The arguments are the developer's command line, passed as written.
        command.raw_arg(arguments);
    }
    crate::log::line(&format!("Running {} {}", program.display(), step.arguments.as_deref().unwrap_or("")));
    let mut child = command.spawn().map_err(|err| {
        crate::log::line(&format!("{} didn't start: {err}", program.display()));
        failed(err.raw_os_error().map_or_else(|| "-1".to_string(), |code| code.to_string()))
    })?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                crate::log::line("It took too long, so setup ended it.");
                return Err(failed("timeout".into()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(150)),
            Err(err) => {
                crate::log::line(&format!("Couldn't wait for it: {err}"));
                return Err(failed("-1".into()));
            }
        }
    };
    let code = status.code().unwrap_or(-1);
    crate::log::line(&format!("It ended with code {code}."));
    if step.succeeded(code) {
        Ok(())
    } else {
        Err(failed(code.to_string()))
    }
}

/// Starts the step's program with administrator rights and waits for it.
fn run_elevated(step: &RunStep, program: &Path, failed: &dyn Fn(String) -> String) -> Result<(), String> {
    let arguments = step.arguments.as_deref().unwrap_or("");
    crate::log::line(&format!("Running {} {arguments} with administrator rights", program.display()));
    match elevate::run(program, arguments, program.parent(), LIMIT, true) {
        Outcome::Exited(code) => {
            // Exit codes are signed on the command line; 0xC0000005 reads as -1073741819.
            let code = code as i32;
            crate::log::line(&format!("It ended with code {code}."));
            if step.succeeded(code) {
                Ok(())
            } else {
                Err(failed(code.to_string()))
            }
        }
        Outcome::Declined => {
            crate::log::line("Windows' administrator prompt was declined.");
            // 1223 is ERROR_CANCELLED, what Windows itself reports for it.
            Err(failed("1223".into()))
        }
        Outcome::TimedOut => {
            crate::log::line("It took too long, so setup ended it.");
            Err(failed("timeout".into()))
        }
        Outcome::Failed(why) => {
            crate::log::line(&format!("{} didn't start: {why}", program.display()));
            Err(failed("-1".into()))
        }
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
        }
    }

    #[test]
    fn a_step_that_succeeds_passes() {
        let dir = folder("ok");
        assert!(run(&step("/c exit 0"), &dir, &texts()).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failing_step_says_which_and_with_what_code() {
        let dir = folder("fail");
        let message = run(&step("/c exit 7"), &dir, &texts()).unwrap_err();
        assert!(message.contains("Helper") && message.contains('7'), "{message}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn listed_exit_codes_count_as_success() {
        let dir = folder("codes");
        let mut step = step("/c exit 3010");
        assert!(run(&step, &dir, &texts()).is_err());
        step.success_codes = vec![0, 3010];
        assert!(run(&step, &dir, &texts()).is_ok(), "3010 is \"restart needed\", which many installers use");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn arguments_reach_the_program_as_written() {
        let dir = folder("args");
        // cmd's own quoting rules show the quotes arrived intact.
        assert!(run(&step("/c \"if \"a\"==\"a\" (exit 0) else (exit 9)\""), &dir, &texts()).is_ok());
        assert!(run(&step("/c \"if \"a\"==\"b\" (exit 0) else (exit 9)\""), &dir, &texts()).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_program_that_is_not_there_fails_to_start() {
        let dir = folder("missing");
        let mut missing = step("");
        missing.file = "bin/not-there.exe".into();
        let message = run(&missing, &dir, &texts()).unwrap_err();
        assert!(message.contains("Helper"), "{message}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn it_runs_in_its_own_folder() {
        let dir = folder("cwd");
        // The marker file only exists next to the program.
        std::fs::write(dir.join("bin/marker.txt"), "x").unwrap();
        assert!(run(&step("/c if exist marker.txt (exit 0) else (exit 5)"), &dir, &texts()).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn manifest_with(steps: Vec<RunStep>) -> Manifest {
        let mut manifest = Package::sample().manifest;
        manifest.files = vec![
            FileEntry::new("sample.exe", 1),
            FileEntry::new("bin/tool.exe", 1),
            FileEntry { path: "extras/more.exe".into(), size: 1, component: Some("extras".into()) },
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
}
