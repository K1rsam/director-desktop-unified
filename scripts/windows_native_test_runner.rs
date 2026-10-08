//! Development-only runner for synthetic tests outside an MSIX parent's file redirection.
//! Compile with the repository's pinned tempfile and serde_json dependencies.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../apps/desktop/src-tauri/src/windows_runtime.rs"]
mod windows_runtime;

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

struct ChildResult {
    exit_code: Option<i32>,
    timed_out: bool,
    pid: u32,
    tree_contained: bool,
    tree_empty: bool,
    job_termination_issued: bool,
    job_processes: Option<u32>,
}

// Attach the job at creation, before the test can spawn its own descendants.
// AssignProcessToJobObject after Command::spawn would leave a startup race.
#[cfg(windows)]
mod contained_child {
    use super::*;
    use std::{
        ffi::{c_void, OsStr},
        mem::size_of,
        os::windows::{ffi::OsStrExt, io::AsRawHandle},
        ptr,
    };

    type Handle = *mut c_void;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x20002;
    const PROC_THREAD_ATTRIBUTE_JOB_LIST: usize = 0x2000d;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        process_time: i64,
        job_time: i64,
        flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Accounting {
        user_time: i64,
        kernel_time: i64,
        period_user_time: i64,
        period_kernel_time: i64,
        faults: u32,
        total: u32,
        active: u32,
        terminated: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct StartupInfo {
        cb: u32,
        reserved: *mut u16,
        desktop: *mut u16,
        title: *mut u16,
        x: u32,
        y: u32,
        x_size: u32,
        y_size: u32,
        x_count: u32,
        y_count: u32,
        fill: u32,
        flags: u32,
        show: u16,
        reserved_size: u16,
        reserved_bytes: *mut u8,
        input: Handle,
        output: Handle,
        error: Handle,
    }
    #[repr(C)]
    struct StartupInfoEx {
        startup: StartupInfo,
        attributes: Handle,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ProcessInfo {
        process: Handle,
        thread: Handle,
        pid: u32,
        tid: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: Handle, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, info: Handle, length: u32) -> i32;
        fn QueryInformationJobObject(
            job: Handle,
            class: i32,
            info: Handle,
            length: u32,
            returned: *mut u32,
        ) -> i32;
        fn TerminateJobObject(job: Handle, code: u32) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn DuplicateHandle(
            source_process: Handle,
            source: Handle,
            target_process: Handle,
            target: *mut Handle,
            access: u32,
            inherit: i32,
            options: u32,
        ) -> i32;
        fn InitializeProcThreadAttributeList(
            list: Handle,
            count: u32,
            flags: u32,
            size: *mut usize,
        ) -> i32;
        fn UpdateProcThreadAttribute(
            list: Handle,
            flags: u32,
            key: usize,
            value: Handle,
            size: usize,
            previous: Handle,
            returned: *mut usize,
        ) -> i32;
        fn DeleteProcThreadAttributeList(list: Handle);
        fn CreateProcessW(
            app: *const u16,
            command: *mut u16,
            pa: Handle,
            ta: Handle,
            inherit: i32,
            flags: u32,
            environment: Handle,
            directory: *const u16,
            startup: *mut StartupInfoEx,
            info: *mut ProcessInfo,
        ) -> i32;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    struct OwnedHandle(Handle);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn checked(ok: i32, operation: &str) -> Result<(), String> {
        if ok == 0 {
            Err(format!("{operation}: {}", std::io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }
    fn inherited(file: &fs::File) -> Result<OwnedHandle, String> {
        let mut handle = ptr::null_mut();
        checked(
            unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    file.as_raw_handle(),
                    GetCurrentProcess(),
                    &mut handle,
                    0,
                    1,
                    2,
                )
            },
            "duplicate test output handle",
        )?;
        Ok(OwnedHandle(handle))
    }
    struct Attributes {
        pointer: Handle,
        _storage: Vec<usize>,
    }
    impl Attributes {
        fn new() -> Result<Self, String> {
            let mut bytes = 0;
            unsafe {
                InitializeProcThreadAttributeList(ptr::null_mut(), 2, 0, &mut bytes);
            }
            if bytes == 0 {
                return Err("missing Windows process attribute storage".into());
            }
            let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            let pointer = storage.as_mut_ptr().cast();
            checked(
                unsafe { InitializeProcThreadAttributeList(pointer, 2, 0, &mut bytes) },
                "initialize test process attributes",
            )?;
            Ok(Self {
                pointer,
                _storage: storage,
            })
        }
        fn set<T>(&mut self, key: usize, value: &mut [T]) -> Result<(), String> {
            checked(
                unsafe {
                    UpdateProcThreadAttribute(
                        self.pointer,
                        0,
                        key,
                        value.as_mut_ptr().cast(),
                        std::mem::size_of_val(value),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    )
                },
                "set test process attributes",
            )
        }
    }
    impl Drop for Attributes {
        fn drop(&mut self) {
            unsafe {
                DeleteProcThreadAttributeList(self.pointer);
            }
        }
    }

    fn quote(value: &OsStr) -> Result<Vec<u16>, String> {
        let mut output = vec![b'"' as u16];
        let mut slashes = 0;
        for unit in value.encode_wide() {
            if unit == 0 {
                return Err("test argument contains NUL".into());
            }
            if unit == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            output.extend(std::iter::repeat_n(
                b'\\' as u16,
                if unit == b'"' as u16 {
                    slashes * 2 + 1
                } else {
                    slashes
                },
            ));
            output.push(unit);
            slashes = 0;
        }
        output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        output.push(b'"' as u16);
        Ok(output)
    }
    fn accounting(job: &OwnedHandle) -> Result<Accounting, String> {
        let mut info = Accounting::default();
        checked(
            unsafe {
                QueryInformationJobObject(
                    job.0,
                    1,
                    (&mut info as *mut Accounting).cast(),
                    size_of::<Accounting>() as u32,
                    ptr::null_mut(),
                )
            },
            "query isolated test job",
        )?;
        Ok(info)
    }
    fn terminate(job: &OwnedHandle) -> Result<(), String> {
        checked(
            unsafe { TerminateJobObject(job.0, 124) },
            "terminate isolated test job",
        )
    }

    pub fn run(
        executable: &Path,
        args: Vec<OsString>,
        stdout: fs::File,
        stderr: fs::File,
        timeout: Duration,
    ) -> Result<ChildResult, String> {
        let job = OwnedHandle(unsafe { CreateJobObjectW(ptr::null_mut(), ptr::null()) });
        if job.0.is_null() {
            return Err(format!(
                "create isolated test job: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut limits = ExtendedLimits::default();
        // No breakaway permission and no inheritable job handle. Dropping the
        // runner, including on errors, also closes the only job handle.
        limits.basic.flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        checked(
            unsafe {
                SetInformationJobObject(
                    job.0,
                    9,
                    (&mut limits as *mut ExtendedLimits).cast(),
                    size_of::<ExtendedLimits>() as u32,
                )
            },
            "set isolated test job limits",
        )?;
        let input_file = fs::File::open("NUL").map_err(|e| e.to_string())?;
        let input = inherited(&input_file)?;
        let output = inherited(&stdout)?;
        let error = inherited(&stderr)?;
        let mut handles = [input.0, output.0, error.0];
        let mut jobs = [job.0];
        let mut attributes = Attributes::new()?;
        attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &mut handles)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &mut jobs)?;
        let app: Vec<_> = executable
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut command = quote(executable.as_os_str())?;
        for arg in args {
            command.push(b' ' as u16);
            command.extend(quote(&arg)?);
        }
        command.push(0);
        if command.len() > 32767 {
            return Err("test command exceeds Windows command-line limit".into());
        }
        let mut startup = StartupInfoEx {
            startup: StartupInfo {
                cb: size_of::<StartupInfoEx>() as u32,
                flags: 0x100,
                input: input.0,
                output: output.0,
                error: error.0,
                ..Default::default()
            },
            attributes: attributes.pointer,
        };
        let mut info = ProcessInfo::default();
        let started = Instant::now();
        // JOB_LIST makes creation and job membership atomic; failure is closed,
        // with no uncontained fallback process. HANDLE_LIST inherits only stdio.
        checked(
            unsafe {
                CreateProcessW(
                    app.as_ptr(),
                    command.as_mut_ptr(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    1,
                    0x08080000,
                    ptr::null_mut(),
                    ptr::null(),
                    &mut startup,
                    &mut info,
                )
            },
            "create contained test process",
        )?;
        let process = OwnedHandle(info.process);
        drop(OwnedHandle(info.thread));
        let mut timed_out = false;
        let mut job_termination_issued = false;
        loop {
            match unsafe { WaitForSingleObject(process.0, 100) } {
                0 => break,
                0x102 => {}
                _ => {
                    return Err(format!(
                        "wait for isolated test process: {}",
                        std::io::Error::last_os_error()
                    ))
                }
            }
            if started.elapsed() >= timeout {
                timed_out = true;
                terminate(&job)?;
                job_termination_issued = true;
                if unsafe { WaitForSingleObject(process.0, 10000) } != 0 {
                    return Err("isolated test process did not terminate after job cleanup".into());
                }
                break;
            }
        }
        let mut exit_code = 0;
        checked(
            unsafe { GetExitCodeProcess(process.0, &mut exit_code) },
            "read test exit code",
        )?;
        drop(process);
        if accounting(&job)?.active != 0 {
            // A successful parent may leave an unreferenced Node/test helper.
            // Its exit does not grant permission to leave that helper running.
            terminate(&job)?;
            job_termination_issued = true;
        }
        let cleanup_started = Instant::now();
        let total = loop {
            let state = accounting(&job)?;
            if state.active == 0 {
                break state.total;
            }
            if cleanup_started.elapsed() >= Duration::from_secs(10) {
                return Err("isolated test job still has active descendants after cleanup".into());
            }
            thread::sleep(Duration::from_millis(10));
        };
        Ok(ChildResult {
            exit_code: Some(exit_code as i32),
            timed_out,
            pid: info.pid,
            tree_contained: true,
            tree_empty: true,
            job_termination_issued,
            job_processes: Some(total),
        })
    }
}

#[cfg(not(windows))]
fn run_child(
    executable: &Path,
    args: Vec<OsString>,
    stdout: fs::File,
    stderr: fs::File,
    timeout: Duration,
) -> Result<ChildResult, String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            child.kill().map_err(|e| e.to_string())?;
            break child.wait().map_err(|e| e.to_string())?;
        }
        thread::sleep(Duration::from_millis(100));
    };
    Ok(ChildResult {
        exit_code: status.code(),
        timed_out,
        pid: child.id(),
        tree_contained: false,
        tree_empty: false,
        job_termination_issued: false,
        job_processes: None,
    })
}

fn run() -> Result<(), String> {
    // The first process exits before creating files. Its Explorer-context child
    // executes this same source boundary and owns the logs/test child.
    if windows_runtime::ensure_unvirtualized()? {
        return Ok(());
    }
    let mut args = std::env::args_os()
        .skip(1)
        .filter(|arg| arg != "--unvirtualized-desktop")
        .peekable();
    let timeout_seconds = if args.peek().is_some_and(|arg| arg == "--timeout-seconds") {
        args.next();
        let value = args
            .next()
            .and_then(|arg| arg.to_str().and_then(|s| s.parse::<u64>().ok()))
            .ok_or("--timeout-seconds requires an integer between 1 and 900")?;
        if !(1..=900).contains(&value) {
            return Err("--timeout-seconds requires an integer between 1 and 900".into());
        }
        value
    } else {
        900
    };
    let output = PathBuf::from(args.next().ok_or("missing absolute evidence directory")?);
    if !output.is_absolute() || output.exists() {
        return Err("evidence directory must be absolute and new".into());
    }
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
        return Err("missing -- before test executable".into());
    }
    let executable = PathBuf::from(args.next().ok_or("missing test executable")?);
    if !executable.is_absolute() || !executable.is_file() {
        return Err("test executable must be an existing absolute file".into());
    }
    if windows_runtime::is_packaged()? {
        return Err("native test runner still has package identity".into());
    }
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    windows_runtime::verify_shared_directory(&output)?;
    let stdout = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("stdout.log"))
        .map_err(|e| e.to_string())?;
    let stderr = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("stderr.log"))
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    #[cfg(windows)]
    let child = contained_child::run(
        &executable,
        args.collect(),
        stdout,
        stderr,
        Duration::from_secs(timeout_seconds),
    )?;
    #[cfg(not(windows))]
    let child = run_child(
        &executable,
        args.collect(),
        stdout,
        stderr,
        Duration::from_secs(timeout_seconds),
    )?;
    let receipt = serde_json::json!({"schemaVersion":"windows-native-test-runner/v1",
        "platform":std::env::consts::OS,"unpackagedRunner":true,"sharedWriteProbeVerified":true,
        "testExecutable":executable,"exitCode":child.exit_code,"success":child.exit_code==Some(0)&&!child.timed_out,
        "timedOut":child.timed_out,"timeoutSeconds":timeout_seconds,"elapsedMilliseconds":started.elapsed().as_millis(),
        "childPid":child.pid,"processTreeContainment":if child.tree_contained {"windows-job-object-at-creation"} else {"direct-child-only"},
        "processTreeEmptyVerified":child.tree_empty,"jobTerminationIssued":child.job_termination_issued,
        "jobProcessesSeen":child.job_processes});
    fs::write(
        output.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        // This tool accepts development arguments only; no account, source text
        // or provider credentials are part of its protocol.
        eprintln!("Windows native synthetic runner failed: {error}");
        std::process::exit(1);
    }
}
