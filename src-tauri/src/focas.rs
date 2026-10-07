#[cfg(target_os = "windows")]
pub use focas_rs::FocasClient;

#[cfg(not(target_os = "windows"))]
pub use mock::FocasClient;

#[cfg(not(target_os = "windows"))]
mod mock {
    use std::collections::HashMap;
    use std::sync::Mutex;

    pub struct FocasClient {
        offsets: Mutex<HashMap<(i16, i16), i32>>,
    }

    pub struct MockData {
        pub data: i32,
    }

    impl FocasClient {
        pub fn new(ip: &str, port: u16) -> anyhow::Result<Self> {
            println!("[Mock CNC] {}:{}: simulated offsets, no hardware connection", ip, port);
            Ok(Self { offsets: Mutex::new(HashMap::new()) })
        }

        pub fn rdtofs(&self, number: i16, kind: i16) -> anyhow::Result<MockData> {
            let offsets = self.offsets.lock().map_err(|_| anyhow::anyhow!("Mock CNC lock poisoned"))?;
            Ok(MockData { data: *offsets.get(&(number, kind)).unwrap_or(&0) })
        }

        pub fn wrtofs(&self, number: i16, kind: i16, offset: i32) -> anyhow::Result<()> {
            let mut offsets = self.offsets.lock().map_err(|_| anyhow::anyhow!("Mock CNC lock poisoned"))?;
            offsets.insert((number, kind), offset);
            println!("[Mock CNC] tool={} type={} offset={}", number, kind, offset);
            Ok(())
        }

        pub fn rdlife(&self, _number: i16) -> anyhow::Result<MockData> {
            Ok(MockData { data: 0 })
        }

        pub fn rdcount(&self, _number: i16) -> anyhow::Result<MockData> {
            Ok(MockData { data: 0 })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn offsets_are_retained_and_isolated() {
            let first = FocasClient::new("dummy", 8193).unwrap();
            let second = FocasClient::new("dummy", 8193).unwrap();
            assert_eq!(first.rdtofs(11, 0).unwrap().data, 0);
            first.wrtofs(11, 0, -12).unwrap();
            assert_eq!(first.rdtofs(11, 0).unwrap().data, -12);
            assert_eq!(first.rdtofs(12, 0).unwrap().data, 0);
            assert_eq!(first.rdtofs(11, 1).unwrap().data, 0);
            assert_eq!(second.rdtofs(11, 0).unwrap().data, 0);
        }
    }
}
