use windows::core::PCWSTR;

use windows::Win32::{
    Foundation::{ERROR_SUCCESS, WIN32_ERROR},
    System::Registry::{
        RegCloseKey, RegCopyTreeW, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW,
        RegEnumValueW, RegOpenKeyExW, RegQueryInfoKeyW, RegQueryValueExW, RegSetValueExW, HKEY,
        KEY_ALL_ACCESS, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_READ, KEY_WRITE, REG_BINARY,
        REG_DWORD, REG_DWORD_BIG_ENDIAN, REG_EXPAND_SZ, REG_MULTI_SZ, REG_NONE,
        REG_OPTION_NON_VOLATILE, REG_OPTION_VOLATILE, REG_QWORD, REG_SZ, REG_VALUE_TYPE,
    },
};

use crate::common::error::win32_error_to_boxed;

pub(crate) fn delete_key_or_value(
    parent: HKEY,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let name_h = windows::core::HSTRING::from(name);

    let res = win32_error_to_boxed(unsafe { RegDeleteTreeW(parent, &name_h) });
    if !res.is_err() {
        return Ok(());
    }

    return win32_error_to_boxed(unsafe { RegDeleteValueW(parent, &name_h) });
}

pub(crate) fn rename_value(
    hkey: HKEY,
    old: &str,
    new: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let old_name = windows::core::HSTRING::from(old);

    let mut value_type = REG_NONE;
    let mut size: u32 = 0;

    win32_error_to_boxed(unsafe {
        RegQueryValueExW(
            hkey,
            &old_name,
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    })?;

    let new_name = windows::core::HSTRING::from(new);

    // 2. Handle empty values
    if size == 0 {
        win32_error_to_boxed(unsafe {
            RegSetValueExW(hkey, &new_name, Some(0), value_type, None)
        })?;
    } else {
        let mut buf = vec![0u8; size as usize];

        win32_error_to_boxed(unsafe {
            RegQueryValueExW(
                hkey,
                &old_name,
                None,
                Some(&mut value_type),
                Some(buf.as_mut_ptr()),
                Some(&mut size),
            )
        })?;

        win32_error_to_boxed(unsafe {
            RegSetValueExW(
                hkey,
                &new_name,
                Some(0),
                value_type,
                Some(&buf[..size as usize]),
            )
        })?;
    }

    return win32_error_to_boxed(unsafe { RegDeleteValueW(hkey, &old_name) });
}

pub(crate) fn rename_key(
    parent: HKEY,
    old: &str,
    new: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let old_name = windows::core::HSTRING::from(old);
    let new_name = windows::core::HSTRING::from(new);

    let mut old_key = HKEY::default();
    let mut new_key = HKEY::default();

    win32_error_to_boxed(unsafe {
        RegOpenKeyExW(parent, &old_name, Some(0), KEY_ALL_ACCESS, &mut old_key)
    })?;

    win32_error_to_boxed(unsafe {
        RegCreateKeyExW(
            parent,
            &new_name,
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS,
            None,
            &mut new_key,
            None,
        )
    })?;

    win32_error_to_boxed(unsafe { RegCopyTreeW(old_key, windows::core::PCWSTR::null(), new_key) })?;

    return win32_error_to_boxed(unsafe { RegDeleteTreeW(parent, &old_name) });
}

pub(crate) fn create_key(
    hkey: HKEY,
    path: &str,
    volatile: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut hkey_out = HKEY::default();
    win32_error_to_boxed(unsafe {
        RegCreateKeyExW(
            hkey,
            PCWSTR(wide.as_ptr()),
            Some(0),
            None,
            if volatile {
                REG_OPTION_VOLATILE
            } else {
                REG_OPTION_NON_VOLATILE
            },
            windows::Win32::System::Registry::KEY_ALL_ACCESS,
            None,
            &mut hkey_out,
            None,
        )
    })?;

    return win32_error_to_boxed(unsafe { RegCloseKey(hkey_out) });
}

pub(crate) fn query_subkey_count(hkey: HKEY) -> Result<u32, Box<dyn std::error::Error>> {
    let mut subkey_count = 0u32;
    win32_error_to_boxed(unsafe {
        RegQueryInfoKeyW(
            hkey,
            Some(windows::core::PWSTR::null()),
            None,
            None,
            Some(&mut subkey_count),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
    })?;
    return Ok(subkey_count);
}

// FIXME: are key ever closed?
// FIXME: Option<&str> -> &str and empty is equivalent to null?
pub(crate) fn open_registry_key(
    hkey: HKEY,
    path: &str,
    writable: bool,
) -> Result<HKEY, Box<dyn std::error::Error>> {
    let mut hkey_out = HKEY::default();

    let subkey = windows::core::PCWSTR::from_raw(windows::core::HSTRING::from(path).as_ptr());
    win32_error_to_boxed(unsafe {
        // Open base hive
        RegOpenKeyExW(
            hkey,
            subkey,
            Some(0),
            if writable {
                KEY_READ | KEY_WRITE
            } else {
                KEY_READ
            },
            &mut hkey_out,
        )
    })?;

    return Ok(hkey_out);
}

pub(crate) fn reg_enum_key(hkey: HKEY, index: u32, name_buf: &mut Vec<u16>) -> WIN32_ERROR {
    name_buf.clear();

    // default size to avoid second call to RegEnumValueW in most cases
    name_buf.resize(260, 0);

    let mut name_len = name_buf.len() as u32;

    let mut status = unsafe {
        RegEnumKeyExW(
            hkey,
            index,
            Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            None,
            Some(windows::core::PWSTR(core::ptr::null_mut())), // check if this works
            None,
            None,
        )
    };
    if status == windows::Win32::Foundation::ERROR_MORE_DATA {
        name_buf.resize(name_len as usize, 0);
        name_len = name_buf.len() as u32;

        status = unsafe {
            RegEnumKeyExW(
                hkey,
                index,
                Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                Some(windows::core::PWSTR(core::ptr::null_mut())), // check if this works
                None,
                None,
            )
        };
    }

    if status != ERROR_SUCCESS {
        name_len = 0;
    }

    name_buf.resize(name_len as usize, 0);
    return status;
}

pub(crate) fn reg_enum_value(
    hkey: HKEY,
    index: u32,
    name_buf: &mut Vec<u16>,
    data_buf: &mut Vec<u8>,
    data_type: &mut REG_VALUE_TYPE,
) -> WIN32_ERROR {
    name_buf.clear();
    data_buf.clear();

    // default size to avoid second call to RegEnumValueW in most cases
    name_buf.resize(260, 0);
    data_buf.resize(4090, 0);

    let mut name_len = name_buf.len() as u32;
    let mut data_len = data_buf.len() as u32;
    *data_type = REG_VALUE_TYPE(0);

    let mut status = unsafe {
        RegEnumValueW(
            hkey,
            index,
            Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            None,
            Some(&mut data_type.0),
            Some(data_buf.as_mut_ptr()),
            Some(&mut data_len),
        )
    };
    if status == windows::Win32::Foundation::ERROR_MORE_DATA {
        name_buf.resize(name_len as usize, 0);
        name_len = name_buf.len() as u32;
        data_buf.resize(data_len as usize, 0);
        data_len = data_buf.len() as u32;

        status = unsafe {
            RegEnumValueW(
                hkey,
                index,
                Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut data_type.0),
                Some(data_buf.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
    }

    if status != ERROR_SUCCESS {
        name_len = 0;
        data_len = 0;
    }

    name_buf.resize(name_len as usize, 0);
    data_buf.resize(data_len as usize, 0);
    return status;
}

pub(crate) fn set_registry_value(
    key: HKEY,
    name_w: &Vec<u16>,
    value_kind: REG_VALUE_TYPE,
    new_value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let data_bytes: Vec<u8> = match value_kind {
        REG_SZ | REG_EXPAND_SZ | REG_MULTI_SZ => {
            let data_w: Vec<u16> = new_value.encode_utf16().chain(Some(0)).collect();
            unsafe { std::slice::from_raw_parts(data_w.as_ptr() as *const u8, data_w.len() * 2) }
                .to_vec()
        }

        REG_NONE | REG_BINARY => new_value.as_bytes().to_vec(),

        REG_DWORD => {
            let new_value = new_value.trim();

            let (radix, digits) = if let Some(hex) = new_value
                .strip_prefix("0x")
                .or_else(|| new_value.strip_prefix("0X"))
            {
                (16, hex)
            } else {
                (10, new_value)
            };

            u32::from_str_radix(digits, radix)
                .map_err(|_| "Invalid numeric value for REG_DWORD")?
                .to_le_bytes()
                .to_vec()
        }

        REG_DWORD_BIG_ENDIAN => {
            let new_value = new_value.trim();

            let (radix, digits) = if let Some(hex) = new_value
                .strip_prefix("0x")
                .or_else(|| new_value.strip_prefix("0X"))
            {
                (16, hex)
            } else {
                (10, new_value)
            };

            u32::from_str_radix(digits, radix)
                .map_err(|_| "Invalid numeric value for REG_DWORD")?
                .to_be_bytes()
                .to_vec()
        }

        REG_QWORD => {
            let new_value = new_value.trim();

            let (radix, digits) = if let Some(hex) = new_value
                .strip_prefix("0x")
                .or_else(|| new_value.strip_prefix("0X"))
            {
                (16, hex)
            } else {
                (10, new_value)
            };

            u64::from_str_radix(digits, radix)
                .map_err(|_| "Invalid numeric value for REG_DWORD")?
                .to_le_bytes()
                .to_vec()
        }

        other => {
            return Err(format!("Unsupported registry type: {:?}", other).into());
        }
    };

    win32_error_to_boxed(unsafe {
        RegSetValueExW(
            key,
            PCWSTR(name_w.as_ptr()),
            Some(0),
            value_kind,
            Some(&data_bytes),
        )
    })?;

    return Ok(());
}
