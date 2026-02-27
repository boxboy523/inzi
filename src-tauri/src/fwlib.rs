use std::{
    ffi::c_uchar,
    os::raw::{c_char, c_long, c_short, c_ulong},
};

use anyhow::anyhow;
use std::sync::{Arc, Mutex, RwLock};

pub type FwlibHndl = c_ulong;

#[repr(C)]
pub struct ODBTOFS {
    pub datano: c_short,
    pub ofs_type: c_short,
    pub data: c_long,
}

#[repr(C)]
pub struct ODBTLIFE3 {
    pub datano: c_short,
    pub dummy: c_short,
    pub data: c_long,
}

#[repr(C)]
pub struct ODBERR {
    pub err_no: c_short,
    pub err_dtno: c_short,
}

#[repr(C)]
pub struct ODBSYS {
    pub dummy: c_short,
    pub max_axis: [c_uchar; 2],
    pub cnc_type: [c_uchar; 2],
    pub mt_type: [c_uchar; 2],
    pub series: [c_uchar; 4],
    pub version: [c_uchar; 4],
    pub axes: [c_uchar; 2],
}

#[derive(Debug)]
pub struct DummyState {
    pub offsets: std::collections::HashMap<i16, i32>,
    pub life: i16,
    pub count: i16,
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

    pub fn cnc_rdlife(flibhndl: FwlibHndl, number: c_short, life: *mut ODBTLIFE3) -> c_short;

    pub fn cnc_rdcount(flibhndl: FwlibHndl, number: c_short, count: *mut ODBTLIFE3) -> c_short;

    pub fn cnc_getdtailerr(flibhndl: FwlibHndl, err: *mut ODBERR) -> c_short;

    pub fn cnc_sysinfo(flibhndl: FwlibHndl, sys: *mut ODBSYS) -> c_short;
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

    pub fn cnc_rdlife(flibhndl: FwlibHndl, number: c_short, life: *mut ODBTLIFE3) -> c_short;

    pub fn cnc_rdcount(flibhndl: FwlibHndl, number: c_short, count: *mut ODBTLIFE3) -> c_short;

    pub fn cnc_getdtailerr(flibhndl: FwlibHndl, err: *mut ODBERR) -> c_short;

    pub fn cnc_sysinfo(flibhndl: FwlibHndl, sys: *mut ODBSYS) -> c_short;

    pub fn cnc_startupprocess(level: c_long, filename: *const c_char) -> c_short;

    pub fn cnc_exitprocess() -> c_short;
}

#[derive(Debug, Clone)]
pub struct FocasClient {
    handle: Arc<Mutex<FwlibHndl>>,
    pub ip: String,
    pub port: i16,
    busy: Arc<RwLock<bool>>,
    dummy_state: Option<Arc<Mutex<DummyState>>>,
}

impl FocasClient {
    pub fn new(ip: &str, port: i16, timeout: i32) -> Result<Self, String> {
        if ip == "dummy" {
            return Ok(FocasClient {
                handle: Arc::new(Mutex::new(0)),
                ip: ip.to_string(),
                port,
                busy: Arc::new(RwLock::new(false)),
                dummy_state: Some(Arc::new(Mutex::new(DummyState {
                    offsets: std::collections::HashMap::new(),
                    life: 100,
                    count: 0,
                }))),
            });
        }

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
                handle: Arc::new(Mutex::new(handle)),
                ip: ip.to_string(),
                port,
                busy: Arc::new(RwLock::new(false)),
                dummy_state: None,
            })
        }
    }

    pub async fn wrtofs(&self, number: i16, ofs_type: i16, data: i32) -> anyhow::Result<()> {
        if self.is_busy() || !self.is_connected() {
            anyhow::bail!("CNC is currently busy with another operation");
        }
        if let Some(dummy) = &self.dummy_state {
            self.set_busy(true);
            let mut state = dummy.lock().unwrap();
            let old_value = state.offsets.get(&number).cloned().unwrap_or(0);
            state.offsets.insert(number, data);
            println!(
                "Dummy write: number={}, ofs_type={}, old_value={}, new_value={}, life={}, count={}",
                number, ofs_type, old_value, data, state.life, state.count
            );
            self.set_busy(false);
            return Ok(());
        }
        loop {
            let current_handle = {
                let guard = self.handle.lock().map_err(|_| {
                    self.set_busy(false);
                    anyhow!("Mutex poisoned")
                })?;
                *guard
            };
            println!(
                "Attempting to write TOFS: number={}, ofs_type={}, data={} to CNC at {}",
                number, ofs_type, data, self.ip
            );
            self.set_busy(true);
            let ret = unsafe {
                let ret = cnc_wrtofs(
                    current_handle,
                    number as c_short,
                    ofs_type as c_short,
                    8,
                    data as c_long,
                );
                if ret != 0 {
                    Err(self.get_error().unwrap_or_else(|e| anyhow!(e.to_string())))
                } else {
                    Ok(())
                }
            };

            if ret.is_ok() {
                self.set_busy(false);
                println!(
                    "Successfully wrote TOFS: number={}, ofs_type={}, data={} to CNC at {}",
                    number, ofs_type, data, self.ip
                );
                return Ok(());
            }

            self.set_busy(false);
            eprintln!(
                "Write failed for CNC at {}:{}. Error: {}.\n Attempting to reconnect...",
                self.ip,
                self.port,
                ret.err().unwrap()
            );
            unsafe {
                cnc_freelibhndl(current_handle);
            }
            {
                let mut guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
                *guard = 0;
            }
            loop {
                let mut new_handle: FwlibHndl = 0;
                let ip_cstr = std::ffi::CString::new(self.ip.as_str()).unwrap();
                let conn_ret = unsafe {
                    cnc_allclibhndl3(ip_cstr.as_ptr(), self.port as c_short, 1, &mut new_handle)
                };
                if conn_ret != 0 {
                    eprintln!(
                        "Reconnection attempt failed for CNC at {}:{}. Error code: {}. Retrying in 5s...",
                        self.ip, self.port, conn_ret
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }

                println!("Successfully reconnected to CNC at {}", self.ip);
                let mut guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
                *guard = new_handle;
                break;
            }
        }
    }

    pub fn rdtofs(&self, number: i16, ofs_type: i16) -> anyhow::Result<ODBTOFS> {
        if self.is_busy() || !self.is_connected() {
            anyhow::bail!("CNC is currently busy with another operation");
        }
        if let Some(dummy) = &self.dummy_state {
            let state = dummy.lock().unwrap();
            let value = state.offsets.get(&number).cloned().unwrap_or(0);
            println!(
                "Dummy read: number={}, ofs_type={}, value={}, life={}, count={}",
                number, ofs_type, value, state.life, state.count
            );
            return Ok(ODBTOFS {
                datano: number as c_short,
                ofs_type: ofs_type as c_short,
                data: value as c_long,
            });
        }
        let current_handle = {
            let guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
            *guard
        };
        println!(
            "Attempting to read TOFS: number={}, ofs_type={} from CNC at {}",
            number, ofs_type, self.ip
        );
        let mut tofs = ODBTOFS {
            datano: 0,
            ofs_type: 0,
            data: 0,
        };
        unsafe {
            let ret = cnc_rdtofs(
                current_handle,
                number as c_short,
                ofs_type as c_short,
                8,
                &mut tofs as *mut ODBTOFS,
            );
            if ret == 0 {
                println!(
                    "Successfully read TOFS: number={}, ofs_type={}, data={} from CNC at {}",
                    number, ofs_type, tofs.data, self.ip
                );
                Ok(tofs)
            } else {
                let err = self.get_error().unwrap_or_else(|e| anyhow!(e.to_string()));
                eprintln!(
                    "Failed to read TOFS: number={}, ofs_type={} from CNC at {}. Error: {}",
                    number, ofs_type, self.ip, err
                );
                Err(anyhow::anyhow!("Failed to read TOFS: {}", err))
            }
        }
    }

    pub fn read_life(&self, number: i16) -> anyhow::Result<i16> {
        if self.is_busy() || !self.is_connected() {
            anyhow::bail!("CNC is currently busy with another operation");
        }
        if let Some(dummy) = &self.dummy_state {
            let state = dummy.lock().unwrap();
            return Ok(state.life);
        }
        let current_handle = {
            let guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
            *guard
        };
        self.set_busy(true);
        let mut life = ODBTLIFE3 {
            datano: 0,
            dummy: 0,
            data: 0,
        };
        unsafe {
            let ret = cnc_rdlife(
                current_handle,
                number as c_short,
                &mut life as *mut ODBTLIFE3,
            );
            self.set_busy(false);
            if ret == 0 {
                Ok(life.data as i16)
            } else {
                let err = self.get_error().unwrap_or_else(|e| anyhow!(e.to_string()));
                eprintln!(
                    "Failed to read life: number={} from CNC at {}. Error: {}",
                    number, self.ip, err
                );
                Err(anyhow::anyhow!("Failed to read life: {}", err))
            }
        }
    }

    pub fn read_count(&self, number: i16) -> anyhow::Result<i16> {
        if self.is_busy() || !self.is_connected() {
            anyhow::bail!("CNC is currently busy with another operation");
        }
        if let Some(dummy) = &self.dummy_state {
            let state = dummy.lock().unwrap();
            return Ok(state.count);
        }
        let current_handle = {
            let guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
            *guard
        };
        self.set_busy(true);
        let mut count = ODBTLIFE3 {
            datano: 0,
            dummy: 0,
            data: 0,
        };
        unsafe {
            let ret = cnc_rdcount(
                current_handle,
                number as c_short,
                &mut count as *mut ODBTLIFE3,
            );
            self.set_busy(false);
            if ret == 0 {
                Ok(count.data as i16)
            } else {
                let err = self.get_error().unwrap_or_else(|e| anyhow!(e.to_string()));
                eprintln!(
                    "Failed to read count: number={} from CNC at {}. Error: {}",
                    number, self.ip, err
                );
                Err(anyhow::anyhow!("Failed to read count: {}", err))
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        if self.dummy_state.is_some() {
            return true;
        }
        match self.handle.lock() {
            Ok(guard) => *guard != 0,
            Err(_) => false,
        }
    }

    pub fn set_busy(&self, busy: bool) {
        let mut guard = self.busy.write().unwrap();
        *guard = busy;
    }

    pub fn is_busy(&self) -> bool {
        let guard = self.busy.read().unwrap();
        *guard
    }

    pub fn get_error(&self) -> anyhow::Result<anyhow::Error> {
        if self.is_busy() || !self.is_connected() {
            anyhow::bail!("CNC is currently busy with another operation");
        }
        if let Some(_) = &self.dummy_state {
            return Ok(anyhow::anyhow!(
                "Dummy client error: no real CNC connection, so no real error details"
            ));
        }
        let current_handle = {
            let guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
            *guard
        };
        let mut err = ODBERR {
            err_no: 0,
            err_dtno: 0,
        };
        unsafe {
            let ret = cnc_getdtailerr(current_handle, &mut err as *mut ODBERR);
            if ret == 0 {
                Ok(anyhow::anyhow!(
                    "CNC Error: err_no={}, err_dtno={}",
                    err.err_no,
                    err.err_dtno
                ))
            } else {
                eprintln!(
                    "Failed to get error details from CNC at {}. Error code: {}",
                    self.ip, ret
                );
                Err(anyhow::anyhow!(
                    "Failed to get error details: error code {}",
                    ret
                ))
            }
        }
    }

    pub fn get_sysinfo(&self) -> anyhow::Result<String> {
        if self.is_busy() || !self.is_connected() {
            anyhow::bail!("CNC is currently busy with another operation");
        }
        if let Some(_) = &self.dummy_state {
            return Ok("Dummy CNC System Info: This is a simulated CNC client with no real hardware connection.".to_string());
        }
        let current_handle = {
            let guard = self.handle.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
            *guard
        };
        let mut sys = ODBSYS {
            dummy: 0,
            max_axis: [0; 2],
            cnc_type: [0; 2],
            mt_type: [0; 2],
            series: [0; 4],
            version: [0; 4],
            axes: [0; 2],
        };
        unsafe {
            let ret = cnc_sysinfo(current_handle, &mut sys as *mut ODBSYS);
            if ret == 0 {
                let max_axis = String::from_utf8_lossy(&sys.max_axis)
                    .trim_matches(char::from(0))
                    .to_string();
                let cnc_type = String::from_utf8_lossy(&sys.cnc_type)
                    .trim_matches(char::from(0))
                    .to_string();
                let mt_type = String::from_utf8_lossy(&sys.mt_type)
                    .trim_matches(char::from(0))
                    .to_string();
                let series = String::from_utf8_lossy(&sys.series)
                    .trim_matches(char::from(0))
                    .to_string();
                let version = String::from_utf8_lossy(&sys.version)
                    .trim_matches(char::from(0))
                    .to_string();
                let axes = String::from_utf8_lossy(&sys.axes)
                    .trim_matches(char::from(0))
                    .to_string();
                Ok(format!(
                    "CNC System Info:\n  Max Axis: {}\n  CNC Type: {}\n  MT Type: {}\n  Series: {}\n  Version: {}\n  Axes: {}",
                    max_axis, cnc_type, mt_type, series, version, axes
                ))
            } else {
                eprintln!(
                    "Failed to get system info from CNC at {}. Error code: {}",
                    self.ip, ret
                );
                Err(anyhow::anyhow!(
                    "Failed to get system info: error code {}",
                    ret
                ))
            }
        }
    }
}

impl Drop for FocasClient {
    fn drop(&mut self) {
        if self.dummy_state.is_some() {
            println!("Dropping dummy client for {}", self.ip);
            return;
        }
        if let Ok(guard) = self.handle.lock() {
            let handle = *guard;
            if handle != 0 {
                println!("Freeing Focas handle for {}", self.ip);
                unsafe {
                    cnc_freelibhndl(handle);
                }
            } else {
                println!("No valid handle to free for {}", self.ip);
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
        let mut handle: FwlibHndl = 0;
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
