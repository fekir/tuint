use windows::core::{Error, HRESULT};
use windows::Win32::Foundation::WIN32_ERROR;

pub(crate) fn win32_error_to_boxed(err: WIN32_ERROR) -> Result<(), Box<dyn std::error::Error>> {
    if err == windows::Win32::Foundation::ERROR_SUCCESS {
        return Ok(());
    }
    let hr = HRESULT(((err.0 & 0xFFFF) | 0x80070000) as i32);
    Err(Box::new(Error::from_hresult(hr)))
}
