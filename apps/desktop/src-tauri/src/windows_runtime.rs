//! A desktop launched by a packaged host must use the same filesystem as an Explorer launch.
//! Run this before creating the webview, project registry, workers, or CLI processes.

#[cfg(windows)]
mod windows {
    use std::{
        ffi::{c_void, OsStr, OsString},
        fs,
        mem::size_of,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            io::AsRawHandle,
        },
        path::Path,
        ptr,
    };
    type Handle = *mut c_void;
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
        fn GetCurrentPackageFamilyName(length: *mut u32, name: *mut u16) -> i32;
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
        fn CloseHandle(handle: Handle) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn GetFinalPathNameByHandleW(
            handle: Handle,
            name: *mut u16,
            length: u32,
            flags: u32,
        ) -> u32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetShellWindow() -> Handle;
        fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
    }
    struct DesktopParent(Handle);
    impl Drop for DesktopParent {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
    fn desktop_parent() -> Result<DesktopParent, String> {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(GetShellWindow(), &mut pid) };
        if pid == 0 {
            return Err("无法取得当前桌面会话，请从桌面启动工作台".into());
        }
        let handle = unsafe { OpenProcess(0x0080, 0, pid) }; // PROCESS_CREATE_PROCESS
        if handle.is_null() {
            return Err(format!(
                "无法使用当前桌面运行环境：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(DesktopParent(handle))
    }
    pub fn is_packaged() -> Result<bool, String> {
        let mut length = 0;
        match unsafe { GetCurrentPackageFamilyName(&mut length, ptr::null_mut()) } {
            15700 => Ok(false), // APPMODEL_ERROR_NO_PACKAGE
            0 | 122 => Ok(true),
            error => Err(format!("无法识别桌面运行环境（Windows {error}）")),
        }
    }
    fn quote(value: &OsStr) -> Vec<u16> {
        let mut out = vec![b'"' as u16];
        let mut slashes = 0;
        for unit in value.encode_wide() {
            if unit == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            out.extend(std::iter::repeat_n(
                b'\\' as u16,
                if unit == b'"' as u16 {
                    slashes * 2 + 1
                } else {
                    slashes
                },
            ));
            out.push(unit);
            slashes = 0;
        }
        out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        out.push(b'"' as u16);
        out
    }
    pub fn ensure_unvirtualized() -> Result<bool, String> {
        // Package identity alone does not prove that inherited file redirection is gone.
        // Use the current session's desktop token/device map once, before any app state opens.
        if std::env::args_os().any(|arg| arg == "--unvirtualized-desktop") {
            return if is_packaged()? {
                Err("Windows 未完成共享桌面环境切换；已停止重启，避免循环启动".into())
            } else {
                Ok(false)
            };
        }
        let parent = desktop_parent()?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe_wide: Vec<_> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut command = quote(exe.as_os_str());
        for argument in std::env::args_os().skip(1) {
            command.push(b' ' as u16);
            command.extend(quote(&argument));
        }
        command.push(b' ' as u16);
        command.extend(quote(OsStr::new("--unvirtualized-desktop")));
        command.push(0);
        let directory = std::env::current_dir().map_err(|e| e.to_string())?;
        let directory_wide: Vec<_> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut bytes = 0;
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 2, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // Windows requires aligned storage for its opaque attribute list.
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let attributes = storage.as_mut_ptr().cast::<c_void>();
        if unsafe { InitializeProcThreadAttributeList(attributes, 2, 0, &mut bytes) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let result = (|| {
            let mut policy: u32 = 1; // PROCESS_CREATION_DESKTOP_APP_BREAKAWAY_ENABLE_PROCESS_TREE
            if unsafe {
                UpdateProcThreadAttribute(
                    attributes,
                    0,
                    0x20012,
                    (&mut policy as *mut u32).cast(),
                    size_of::<u32>(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut parent_handle = parent.0;
            if unsafe {
                UpdateProcThreadAttribute(
                    attributes,
                    0,
                    0x20000, // PROC_THREAD_ATTRIBUTE_PARENT_PROCESS
                    (&mut parent_handle as *mut Handle).cast(),
                    size_of::<Handle>(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut startup = StartupInfoEx {
                startup: StartupInfo {
                    cb: size_of::<StartupInfoEx>() as u32,
                    ..Default::default()
                },
                attributes,
            };
            let mut info = ProcessInfo::default();
            if unsafe {
                CreateProcessW(
                    exe_wide.as_ptr(),
                    command.as_mut_ptr(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    0x08080000,
                    ptr::null_mut(),
                    directory_wide.as_ptr(),
                    &mut startup,
                    &mut info,
                )
            } == 0
            {
                return Err(format!(
                    "无法以共享项目环境启动桌面端：{}",
                    std::io::Error::last_os_error()
                ));
            }
            unsafe {
                CloseHandle(info.thread);
                CloseHandle(info.process);
            }
            Ok(true)
        })();
        unsafe {
            DeleteProcThreadAttributeList(attributes);
        }
        result
    }

    fn opened_file_path(file: &fs::File) -> Result<std::path::PathBuf, String> {
        let mut name = vec![0u16; 32768];
        let length = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle(), name.as_mut_ptr(), name.len() as u32, 0)
        };
        if length == 0 || length as usize >= name.len() {
            return Err(format!("无法核对项目文件的实际路径：{}", std::io::Error::last_os_error()));
        }
        Ok(std::path::PathBuf::from(OsString::from_wide(&name[..length as usize])))
    }

    /// Verify existing reads and a new-file write in this application's own home.
    /// MSIX can redirect individual files even when package-identity APIs return no package.
    pub fn verify_shared_directory(home: &Path) -> Result<(), String> {
        if is_packaged()? {
            return Err("项目存储仍在宿主隔离环境，未启动执行器".into());
        }
        fs::create_dir_all(home).map_err(|e| e.to_string())?;
        let physical_home = fs::canonicalize(home).map_err(|e| e.to_string())?;
        // Checking only an existing registry says nothing about the first future
        // write. This owned probe is immediately removed; no project is opened.
        let probe = tempfile::Builder::new()
            .prefix(".director-native-path-probe-")
            .tempfile_in(home)
            .map_err(|e| e.to_string())?;
        let expected = physical_home.join(probe.path().file_name().ok_or("路径核验文件没有名称")?);
        if !same_windows_path(&expected, &opened_file_path(probe.as_file())?) {
            return Err("Windows 仍在重定向新项目文件，未初始化项目或启动执行器".into());
        }
        let registry = home.join("projects.json");
        if registry.is_file() {
            let expected = physical_home.join("projects.json");
            // Open with the same read access as the registry reader; attribute-only metadata
            // lookups can describe the unredirected directory while reads use a shadow file.
            let file = fs::File::open(&registry).map_err(|e| e.to_string())?;
            let actual = opened_file_path(&file)?;
            if !same_windows_path(&expected, &actual) {
                return Err("Windows 仍在重定向项目登记，未读取隔离列表或启动执行器".into());
            }
        }
        Ok(())
    }
    fn same_windows_path(left: &Path, right: &Path) -> bool {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn windows_arguments_preserve_spaces_quotes_and_trailing_slashes() {
            let rendered = |text: &str| String::from_utf16(&quote(OsStr::new(text))).unwrap();
            assert_eq!(rendered(""), "\"\"");
            assert_eq!(rendered("项目 1"), "\"项目 1\"");
            assert_eq!(rendered("a\\\"b"), "\"a\\\\\\\"b\"");
            assert_eq!(rendered("C:\\项目\\"), "\"C:\\项目\\\\\"");
        }
        #[test]
        fn shared_registry_does_not_accept_another_filesystem_directory() {
            assert!(same_windows_path(
                Path::new(r"\\?\C:\Users\A\Projects\projects.json"),
                Path::new(r"\\?\c:\users\a\projects\PROJECTS.json")
            ));
            assert!(!same_windows_path(
                Path::new(r"\\?\C:\Users\A\AppData\Roaming\workbench\projects.json"),
                Path::new(
                    r"\\?\C:\Users\A\AppData\Local\Packages\Host\LocalCache\Roaming\workbench\projects.json"
                )
            ));
            let home = tempfile::tempdir().unwrap();
            fs::write(home.path().join("projects.json"), b"[]").unwrap();
            // A real file/directory handle round trip, without opening production data.
            assert!(!is_packaged().unwrap(), "native path regression requires an unpackaged process");
            verify_shared_directory(home.path()).unwrap();
            assert_eq!(fs::read_dir(home.path()).unwrap().count(), 1, "owned write probe must be removed");
        }
        #[test]
        fn empty_home_checks_future_writes_without_initializing_a_registry() {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("中文 项目目录");
            assert!(!is_packaged().unwrap(), "native write regression requires an unpackaged process");
            verify_shared_directory(&home).unwrap();
            assert!(home.is_dir());
            assert!(!home.join("projects.json").exists());
            assert_eq!(fs::read_dir(home).unwrap().count(), 0);
        }
    }
}

#[cfg(windows)]
pub use windows::{ensure_unvirtualized, is_packaged, verify_shared_directory};
#[cfg(not(windows))]
pub fn ensure_unvirtualized() -> Result<bool, String> {
    Ok(false)
}
#[cfg(not(windows))]
pub fn is_packaged() -> Result<bool, String> {
    Ok(false)
}
#[cfg(not(windows))]
pub fn verify_shared_directory(_: &std::path::Path) -> Result<(), String> {
    Ok(())
}
