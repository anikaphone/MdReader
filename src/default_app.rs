#[cfg(windows)]
use std::ffi::{c_void, OsStr};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(windows)]
type HKEY = *mut c_void;
#[cfg(windows)]
const HKEY_CURRENT_USER: HKEY = -2147483647i32 as HKEY;
#[cfg(windows)]
const KEY_WRITE: u32 = 0x20006;
#[cfg(windows)]
const KEY_READ: u32 = 0x20019;
#[cfg(windows)]
const REG_SZ: u32 = 1;

#[cfg(windows)]
#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        hKey: HKEY,
        lpSubKey: *const u16,
        Reserved: u32,
        lpClass: *mut u16,
        dwOptions: u32,
        samDesired: u32,
        lpSecurityAttributes: *mut c_void,
        phkResult: *mut HKEY,
        lpdwDisposition: *mut u32,
    ) -> i32;

    fn RegOpenKeyExW(
        hKey: HKEY,
        lpSubKey: *const u16,
        ulOptions: u32,
        samDesired: u32,
        phkResult: *mut HKEY,
    ) -> i32;

    fn RegSetValueExW(
        hKey: HKEY,
        lpValueName: *const u16,
        Reserved: u32,
        dwType: u32,
        lpData: *const u8,
        cbData: u32,
    ) -> i32;

    fn RegQueryValueExW(
        hKey: HKEY,
        lpValueName: *const u16,
        lpReserved: *mut u32,
        lpType: *mut u32,
        lpData: *mut u8,
        lpcbData: *mut u32,
    ) -> i32;

    fn RegCloseKey(hKey: HKEY) -> i32;
}

#[cfg(windows)]
#[link(name = "shell32")]
extern "system" {
    fn SHChangeNotify(
        wEventId: i32,
        uFlags: u32,
        dwItem1: *const c_void,
        dwItem2: *const c_void,
    );
}

#[cfg(windows)]
const SHCNE_ASSOCCHANGED: i32 = 0x08000000;
#[cfg(windows)]
const SHCNF_IDLIST: u32 = 0x0000;

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn set_reg_value(root: HKEY, subkey: &str, value_name: Option<&str>, value: &str) -> Result<(), String> {
    let subkey_w = to_wide(subkey);
    let mut hkey: HKEY = std::ptr::null_mut();
    let res = unsafe {
        RegCreateKeyExW(
            root,
            subkey_w.as_ptr(),
            0,
            std::ptr::null_mut(),
            0,
            KEY_WRITE,
            std::ptr::null_mut(),
            &mut hkey,
            std::ptr::null_mut(),
        )
    };
    if res != 0 {
        return Err(format!("创建注册表项失败 '{}': 错误代码 {}", subkey, res));
    }

    let val_w = to_wide(value);
    let val_name_w = value_name.map(to_wide);
    let val_name_ptr = val_name_w.as_ref().map(|v| v.as_ptr()).unwrap_or(std::ptr::null());

    let set_res = unsafe {
        RegSetValueExW(
            hkey,
            val_name_ptr,
            0,
            REG_SZ,
            val_w.as_ptr() as *const u8,
            (val_w.len() * std::mem::size_of::<u16>()) as u32,
        )
    };
    unsafe {
        RegCloseKey(hkey);
    }

    if set_res != 0 {
        return Err(format!("写入注册表值失败 '{}': 错误代码 {}", subkey, set_res));
    }
    Ok(())
}

#[cfg(windows)]
pub fn is_default_md_reader() -> bool {
    let subkey_w = to_wide(r"Software\Classes\.md");
    let mut hkey: HKEY = std::ptr::null_mut();
    let res = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey_w.as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        )
    };
    if res != 0 {
        return false;
    }

    let mut buf = [0u16; 128];
    let mut len = (buf.len() * 2) as u32;
    let mut val_type = 0u32;
    let query_res = unsafe {
        RegQueryValueExW(
            hkey,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut val_type,
            buf.as_mut_ptr() as *mut u8,
            &mut len,
        )
    };
    unsafe {
        RegCloseKey(hkey);
    }

    if query_res == 0 && val_type == REG_SZ {
        let actual_len = (len as usize / 2).saturating_sub(1);
        let s = String::from_utf16_lossy(&buf[..actual_len]);
        s == "MdReader.Document"
    } else {
        false
    }
}

#[cfg(windows)]
pub fn set_as_default_md_reader() -> Result<(), String> {
    let exe_path = std::env::current_exe().map_err(|e| format!("获取程序路径失败: {}", e))?;
    let exe_str = exe_path.to_str().ok_or_else(|| "程序路径包含无效字符".to_string())?;

    let prog_id = "MdReader.Document";
    let app_name = "mdreader.exe";
    let icon_val = format!("\"{}\",0", exe_str);
    let open_cmd = format!("\"{}\" \"%1\"", exe_str);

    // 1. ProgID 定义: HKCU\Software\Classes\MdReader.Document
    set_reg_value(HKEY_CURRENT_USER, &format!(r"Software\Classes\{}", prog_id), None, "Markdown 文档")?;
    set_reg_value(HKEY_CURRENT_USER, &format!(r"Software\Classes\{}\DefaultIcon", prog_id), None, &icon_val)?;
    set_reg_value(HKEY_CURRENT_USER, &format!(r"Software\Classes\{}\shell\open\command", prog_id), None, &open_cmd)?;
    set_reg_value(HKEY_CURRENT_USER, &format!(r"Software\Classes\{}\shell\open", prog_id), Some("FriendlyAppName"), "MdReader")?;

    // 2. 应用程序定义: HKCU\Software\Classes\Applications\mdreader.exe
    let app_key = format!(r"Software\Classes\Applications\{}", app_name);
    set_reg_value(HKEY_CURRENT_USER, &app_key, Some("FriendlyAppName"), "MdReader")?;
    set_reg_value(HKEY_CURRENT_USER, &format!(r"{}\DefaultIcon", app_key), None, &icon_val)?;
    set_reg_value(HKEY_CURRENT_USER, &format!(r"{}\shell\open\command", app_key), None, &open_cmd)?;

    let extensions = [".md", ".markdown", ".mdown", ".mkd"];
    for ext in &extensions {
        set_reg_value(HKEY_CURRENT_USER, &format!(r"{}\SupportedTypes", app_key), Some(ext), "")?;

        // HKCU\Software\Classes\<ext>
        set_reg_value(HKEY_CURRENT_USER, &format!(r"Software\Classes\{}", ext), None, prog_id)?;
        set_reg_value(HKEY_CURRENT_USER, &format!(r"Software\Classes\{}\OpenWithProgids", ext), Some(prog_id), "")?;
        set_reg_value(HKEY_CURRENT_USER, &format!(r"Software\Classes\{}\OpenWithList\{}", ext, app_name), None, "")?;

        // Explorer FileExts OpenWithProgids & OpenWithList
        let explorer_ext = format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{}", ext);
        set_reg_value(HKEY_CURRENT_USER, &format!(r"{}\OpenWithProgids", explorer_ext), Some(prog_id), "")?;
        set_reg_value(HKEY_CURRENT_USER, &format!(r"{}\OpenWithList", explorer_ext), Some("a"), app_name)?;
        set_reg_value(HKEY_CURRENT_USER, &format!(r"{}\OpenWithList", explorer_ext), Some("MRUList"), "a")?;
    }

    // 3. 注册 Windows 默认程序功能能力 (Default Programs Capabilities)
    set_reg_value(HKEY_CURRENT_USER, r"Software\MdReader\Capabilities", Some("ApplicationName"), "MdReader")?;
    set_reg_value(HKEY_CURRENT_USER, r"Software\MdReader\Capabilities", Some("ApplicationDescription"), "极速纯绿色原生 Markdown 阅读器")?;
    for ext in &extensions {
        set_reg_value(HKEY_CURRENT_USER, r"Software\MdReader\Capabilities\FileAssociations", Some(ext), prog_id)?;
    }
    set_reg_value(HKEY_CURRENT_USER, r"Software\RegisteredApplications", Some("MdReader"), r"Software\MdReader\Capabilities")?;

    // 4. 通知 Windows Shell / Explorer 文件关联已发生改变
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, std::ptr::null(), std::ptr::null());
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn is_default_md_reader() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_as_default_md_reader() -> Result<(), String> {
    Err("当前仅支持在 Windows 系统下设置默认打开方式".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_wide() {
        let wide = to_wide("hello");
        assert_eq!(wide, vec!['h' as u16, 'e' as u16, 'l' as u16, 'l' as u16, 'o' as u16, 0]);
    }
}
