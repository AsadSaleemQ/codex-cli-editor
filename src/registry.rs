#[cfg(windows)]
mod implementation {
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;

    use winapi::shared::minwindef::{DWORD, HKEY, LPARAM, LPBYTE, WPARAM};
    use winapi::shared::winerror::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use winapi::um::winnt::{KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ};
    use winapi::um::winreg::{
        HKEY_CURRENT_USER, RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW,
    };
    use winapi::um::winuser::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW};

    use crate::error::Result;
    use crate::{CodexCliEditorError, RegistryValueSnapshot};

    const ENVIRONMENT_KEY: &str = "Environment";
    const PATH_VALUE: &str = "Path";

    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: Key owns the successfully opened registry handle.
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain([0]).collect()
    }

    fn registry_error(api: &'static str, status: i32) -> CodexCliEditorError {
        CodexCliEditorError::RegistryApi {
            api,
            source: std::io::Error::from_raw_os_error(status),
        }
    }

    fn open_environment(access: DWORD) -> Result<Key> {
        let name = wide(ENVIRONMENT_KEY);
        let mut key = null_mut();
        // SAFETY: name is NUL terminated and key is a writable output pointer.
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut key) };
        if status != ERROR_SUCCESS as i32 {
            return Err(registry_error("RegOpenKeyExW", status));
        }
        Ok(Key(key))
    }

    pub fn read_user_path() -> Result<RegistryValueSnapshot> {
        let key = open_environment(KEY_QUERY_VALUE)?;
        let name = wide(PATH_VALUE);
        let mut value_type = 0u32;
        let mut size = 0u32;
        // SAFETY: output pointers are valid; null data requests the required size.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                null_mut(),
                &mut value_type,
                null_mut(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND as i32 {
            return Ok(RegistryValueSnapshot {
                existed: false,
                value_type: REG_EXPAND_SZ,
                data: Vec::new(),
            });
        }
        if status != ERROR_SUCCESS as i32 {
            return Err(registry_error("RegQueryValueExW", status));
        }
        if value_type != REG_SZ && value_type != REG_EXPAND_SZ
            || !size.is_multiple_of(size_of::<u16>() as u32)
        {
            return Err(CodexCliEditorError::UnsupportedUserPath);
        }
        let mut data = vec![0u8; size as usize];
        // SAFETY: data has exactly the byte capacity reported by the first query.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                null_mut(),
                &mut value_type,
                data.as_mut_ptr(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS as i32 {
            return Err(registry_error("RegQueryValueExW", status));
        }
        data.truncate(size as usize);
        Ok(RegistryValueSnapshot {
            existed: true,
            value_type,
            data,
        })
    }

    pub fn prepend_shim(snapshot: &RegistryValueSnapshot, shim: &Path) -> Result<Vec<u8>> {
        let mut units = Vec::with_capacity(snapshot.data.len() / 2);
        for bytes in snapshot.data.as_chunks::<2>().0 {
            units.push(u16::from_le_bytes(*bytes));
        }
        while units.last() == Some(&0) {
            units.pop();
        }
        let current =
            String::from_utf16(&units).map_err(|_| CodexCliEditorError::UnsupportedUserPath)?;
        let shim_text = shim.as_os_str().to_string_lossy();
        if current
            .split(';')
            .any(|entry| entry.eq_ignore_ascii_case(&shim_text))
        {
            return Ok(snapshot.data.clone());
        }
        let mut next: Vec<u16> = shim.as_os_str().encode_wide().collect();
        if !units.is_empty() {
            next.push(b';' as u16);
            next.extend(units);
        }
        next.push(0);
        Ok(next.into_iter().flat_map(u16::to_le_bytes).collect())
    }

    pub fn remove_shim(snapshot: &RegistryValueSnapshot, shim: &Path) -> Result<Vec<u8>> {
        let mut units = Vec::with_capacity(snapshot.data.len() / 2);
        for bytes in snapshot.data.as_chunks::<2>().0 {
            units.push(u16::from_le_bytes(*bytes));
        }
        while units.last() == Some(&0) {
            units.pop();
        }
        let current =
            String::from_utf16(&units).map_err(|_| CodexCliEditorError::UnsupportedUserPath)?;
        let shim_text = shim.as_os_str().to_string_lossy();
        let filtered = current
            .split(';')
            .filter(|entry| !entry.eq_ignore_ascii_case(&shim_text))
            .collect::<Vec<_>>()
            .join(";");
        if filtered == current {
            return Ok(snapshot.data.clone());
        }
        let mut next: Vec<u16> = filtered.encode_utf16().collect();
        next.push(0);
        Ok(next.into_iter().flat_map(u16::to_le_bytes).collect())
    }

    pub fn write_user_path(value_type: DWORD, data: &[u8]) -> Result<()> {
        if value_type != REG_SZ && value_type != REG_EXPAND_SZ {
            return Err(CodexCliEditorError::UnsupportedUserPath);
        }
        let key = open_environment(KEY_SET_VALUE)?;
        let name = wide(PATH_VALUE);
        // SAFETY: key is writable, name is terminated, and data is valid for its byte length.
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                value_type,
                data.as_ptr() as LPBYTE,
                data.len() as DWORD,
            )
        };
        if status != ERROR_SUCCESS as i32 {
            return Err(registry_error("RegSetValueExW", status));
        }
        if let Err(error) = broadcast_environment_change() {
            eprintln!(
                "warning: user PATH changed but Windows environment broadcast failed: {error}"
            );
        }
        Ok(())
    }

    fn broadcast_environment_change() -> Result<()> {
        const WM_SETTINGCHANGE: u32 = 0x001A;
        let environment = wide("Environment");
        let mut result = 0usize;
        // SAFETY: the broadcast is synchronous for at most five seconds and the UTF-16 buffer
        // remains alive for the duration of the call.
        let sent = unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0 as WPARAM,
                environment.as_ptr() as LPARAM,
                SMTO_ABORTIFHUNG,
                5_000,
                &mut result,
            )
        };
        if sent == 0 {
            return Err(CodexCliEditorError::WindowsApi {
                api: "SendMessageTimeoutW",
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(())
    }
    pub fn restore_user_path(snapshot: &RegistryValueSnapshot) -> Result<()> {
        if snapshot.existed {
            return write_user_path(snapshot.value_type, &snapshot.data);
        }
        let key = open_environment(KEY_SET_VALUE)?;
        let name = wide(PATH_VALUE);
        // SAFETY: key is writable and name is NUL terminated.
        let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        if status != ERROR_SUCCESS as i32 && status != ERROR_FILE_NOT_FOUND as i32 {
            return Err(registry_error("RegDeleteValueW", status));
        }
        if let Err(error) = broadcast_environment_change() {
            eprintln!(
                "warning: user PATH changed but Windows environment broadcast failed: {error}"
            );
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use implementation::{
    prepend_shim, read_user_path, remove_shim, restore_user_path, write_user_path,
};
