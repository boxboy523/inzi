use std::os::raw::{c_char, c_long, c_short, c_ushort};

use std::sync::Mutex;

use anyhow::anyhow;

pub type FwlibHndl = c_ushort;

#[repr(C)]
pub struct ODBTOFS {
    pub datano: c_short,
    pub ofs_type: c_short,
    pub data: c_long,
}

#[cfg(target_os = "windows")]
#[link(name = "Fwlib64")]
extern "C" {
    fn cnc_allclibhndl3(
        ip_addr: *const c_char,
        port: c_short,
        timeout: c_long,
        flibhndl: *mut FwlibHndl,
    ) -> c_short;

    fn cnc_freelibhndl(flibhndl: FwlibHndl) -> c_short;

    fn cnc_rdtofs(
        flibhndl: FwlibHndl,
        number: c_short,
        ofs_type: c_short,
        length: c_short,
        tofs: *mut ODBTOFS,
    ) -> c_short;

    fn cnc_wrtofs(
        flibhndl: FwlibHndl,
        number: c_short,
        ofs_type: c_short,
        length: c_short,
        data: c_long,
    ) -> c_short;
}

#[cfg(target_os = "linux")]
#[link(name = "fwlib32")]
extern "C" {
    fn cnc_allclibhndl3(
        ip_addr: *const c_char,
        port: c_short,
        timeout: c_long,
        flibhndl: *mut FwlibHndl,
    ) -> c_short;

    fn cnc_freelibhndl(flibhndl: FwlibHndl) -> c_short;

    fn cnc_rdtofs(
        flibhndl: FwlibHndl,
        number: c_short,
        ofs_type: c_short,
        length: c_short,
        tofs: *mut ODBTOFS,
    ) -> c_short;

    fn cnc_wrtofs(
        flibhndl: FwlibHndl,
        number: c_short,
        ofs_type: c_short,
        length: c_short,
        data: c_long,
    ) -> c_short;

    pub fn cnc_startupprocess(level: c_long, filename: *const c_char) -> c_short;

    pub fn cnc_exitprocess() -> c_short;
}

pub struct FocasClient {
    handle: Mutex<FwlibHndl>,
    ip: String,
    port: i16,
}

impl FocasClient {
    pub fn new(ip: &str, port: i16, timeout: i32) -> Result<Self, String> {
        let c_ip = std::ffi::CString::new(ip).unwrap();
        let mut handle: FwlibHndl = 0;

        let ret = unsafe {
            cnc_allclibhndl3(
                c_ip.as_ptr(),
                port as c_short,
                timeout as c_long,
                &mut handle,
            )
        };

        if ret != 0 {
            Err(format!("Failed to allocate handle: error code {}", ret))
        } else {
            Ok(FocasClient {
                handle: Mutex::new(handle),
                ip: ip.to_string(),
                port,
            })
        }
    }

    pub async fn wrtofs(&self, number: i16, ofs_type: i16, data: i32) -> anyhow::Result<()> {
        loop {
            let current_handle = {
                let guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
                *guard
            };
            let ret = unsafe {
                cnc_wrtofs(
                    current_handle,
                    number as c_short,
                    ofs_type as c_short,
                    8,
                    data as c_long,
                )
            };

            if ret == 0 {
                return Ok(());
            }

            eprintln!(
                "CNC Write Error (Code: {}). Attempting to reconnect...",
                ret,
            );

            unsafe {
                cnc_freelibhndl(current_handle);
            }

            loop {
                let mut new_handle: u16 = 0;
                let ip_cstr = std::ffi::CString::new(self.ip.as_str()).unwrap();

                let conn_ret = unsafe {
                    cnc_allclibhndl3(ip_cstr.as_ptr(), self.port as c_short, 1, &mut new_handle)
                };

                if conn_ret == 0 {
                    println!("Successfully reconnected to CNC at {}", self.ip);
                    let mut guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
                    *guard = new_handle;
                    break; // 재연결 성공했으므로 쓰기 시도로 돌아감
                }

                eprintln!(
                    "Reconnection failed (Code: {}). Retrying in 5s...",
                    conn_ret
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    }

    pub fn rdtofs(&self, number: i16, ofs_type: i16) -> anyhow::Result<ODBTOFS> {
        let mut tofs = ODBTOFS {
            datano: 0,
            ofs_type: 0,
            data: 0,
        };
        let current_handle = {
            let guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
            *guard
        };
        let ret = unsafe {
            cnc_rdtofs(
                current_handle,
                number as c_short,
                ofs_type as c_short,
                8,
                &mut tofs as *mut ODBTOFS,
            )
        };
        if ret != 0 {
            Err(anyhow::anyhow!("Failed to read TOFS: error code {}", ret))
        } else {
            Ok(tofs)
        }
    }

    pub fn is_connected(&self) -> bool {
        match self.handle.lock() {
            Ok(guard) => *guard != 0,
            Err(_) => false,
        }
    }
}

impl Drop for FocasClient {
    fn drop(&mut self) {
        if let Ok(guard) = self.handle.lock() {
            unsafe {
                cnc_freelibhndl(*guard);
            }
        }
    }
}

#[cfg(test)]
mod focas_tests {
    use super::*; // extern "C" 선언이 있는 곳
    use std::ffi::CString;

    #[test]
    fn test_focas_library_linkage() {
        // 1. 가짜 IP 주소 (연결이 안 되어야 정상)
        #[cfg(target_os = "linux")]
        {
            let log_file = CString::new("focas2.log").unwrap();
            let init_ret = unsafe { cnc_startupprocess(3, log_file.as_ptr()) };
            println!("리눅스 로그 초기화 결과 코드: {}", init_ret);
        }

        let ip = CString::new("127.0.0.1").unwrap();
        let mut handle: u16 = 0;
        let ret = unsafe { cnc_allclibhndl3(ip.as_ptr(), 8193, 3, &mut handle) };

        println!("FOCAS 함수 호출 결과 코드: {}", ret);

        assert_ne!(ret, 0, "장비가 없는데 연결이 성공할 리 없음");

        println!("✅ 라이브러리 로딩 및 함수 링킹 성공!");
        #[cfg(target_os = "linux")]
        unsafe {
            cnc_exitprocess();
        }
    }
}
