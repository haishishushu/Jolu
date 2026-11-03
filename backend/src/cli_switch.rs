//! 所有读写 agent 配置的入口共用的一把锁。
//!
//! 进程内用 `Mutex`，进程间用 Windows 命名互斥量：正式版、验收版、改名测试包可能共用
//! 同一份 `~/.claude` / `~/.codex`，两个 Jolu 同时改同一个文件时后写的会覆盖先写的。
//! 等待有上限：另一个实例卡住时返回可读错误，不无限阻塞设置页。

use std::sync::Mutex;

const NAMED_MUTEX: &str = "jolu-wiring-agent-config";

/// 进程内 + 跨进程串行执行配置写入。
pub(crate) fn with_switch<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().map_err(|_| "CLI 配置切换锁不可用，请重启应用")?;
    let _named = NamedGuard::acquire()?;
    work()
}

#[cfg(windows)]
struct NamedGuard(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl NamedGuard {
    fn acquire() -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};

        let open = |name: &str| -> Result<windows_sys::Win32::Foundation::HANDLE, String> {
            let wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().chain(std::iter::once(0)).collect();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
            if handle.is_null() {
                Err(format!("无法创建配置锁（{name}）"))
            } else {
                Ok(handle)
            }
        };
        // Global 命名空间需要跨会话；没有权限时退回当前会话，至少挡住同一用户下的多个实例。
        let handle = open(&format!("Global\\{NAMED_MUTEX}")).or_else(|_| open(&format!("Local\\{NAMED_MUTEX}")))?;
        let waited = unsafe { WaitForSingleObject(handle, 15_000) };
        match waited {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle)),
            WAIT_TIMEOUT => {
                unsafe { CloseHandle(handle) };
                Err("另一个 Jolu 正在修改工具配置，请稍后再试".into())
            }
            _ => {
                unsafe { CloseHandle(handle) };
                Err("等待配置锁失败，请重启应用".into())
            }
        }
    }
}

#[cfg(windows)]
impl Drop for NamedGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::ReleaseMutex;
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

#[cfg(not(windows))]
struct NamedGuard;

#[cfg(not(windows))]
impl NamedGuard {
    fn acquire() -> Result<Self, String> {
        Ok(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_serializes_and_returns_value() {
        assert_eq!(with_switch(|| Ok(7)).unwrap(), 7);
        assert!(with_switch(|| Err::<(), _>("boom".to_string())).is_err());
        // 释放后还能再次获取（Drop 正确归还互斥量）
        assert_eq!(with_switch(|| Ok(8)).unwrap(), 8);
    }
}
