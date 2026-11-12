//! The signed-in PalDuels session, encrypted at rest the way each system expects:
//! - macOS: AES-256-GCM, with the key kept in the login Keychain (like Electron's safeStorage). The encrypted
//!   session is a file next to the database; without the Keychain item it can't be read.
//! - Windows: DPAPI (CryptProtectData), tied to the Windows user account.
//! - Elsewhere: not stored (the account features need one of the above).
//!
//! Only the session cookie is stored. Augur never sees or keeps a password.

use std::path::{Path, PathBuf};

fn file(dir: &Path) -> PathBuf {
    dir.join("session.bin")
}

pub fn store(dir: &Path, secret: &str) -> Result<(), String> {
    let blob = imp::encrypt(secret.as_bytes())?;
    std::fs::write(file(dir), blob).map_err(|e| e.to_string())
}

pub fn load(dir: &Path) -> Option<String> {
    let blob = std::fs::read(file(dir)).ok()?;
    String::from_utf8(imp::decrypt(&blob).ok()?).ok()
}

pub fn clear(dir: &Path) {
    let _ = std::fs::remove_file(file(dir));
    imp::forget();
}

#[cfg(target_os = "macos")]
mod imp {
    use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
    use aes_gcm::{Aes256Gcm, Key, Nonce};
    use base64::Engine;

    const SERVICE: &str = "Augur Safe Storage";
    const ACCOUNT: &str = "Augur";

    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())
    }

    /// The Keychain-held key, made on first use.
    fn key() -> Result<Key<Aes256Gcm>, String> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let e = entry()?;
        match e.get_password() {
            Ok(k) => {
                let raw = b64.decode(k).map_err(|e| e.to_string())?;
                (raw.len() == 32).then(|| *Key::<Aes256Gcm>::from_slice(&raw)).ok_or_else(|| "bad key in Keychain".to_string())
            }
            Err(keyring::Error::NoEntry) => {
                let k = Aes256Gcm::generate_key(OsRng);
                e.set_password(&b64.encode(k)).map_err(|e| e.to_string())?;
                Ok(k)
            }
            Err(err) => Err(err.to_string()),
        }
    }

    pub fn encrypt(plain: &[u8]) -> Result<Vec<u8>, String> {
        let cipher = Aes256Gcm::new(&key()?);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut out = nonce.to_vec();
        out.extend(cipher.encrypt(&nonce, plain).map_err(|e| e.to_string())?);
        Ok(out)
    }

    pub fn decrypt(blob: &[u8]) -> Result<Vec<u8>, String> {
        if blob.len() < 12 {
            return Err("too short".into());
        }
        let cipher = Aes256Gcm::new(&key()?);
        cipher.decrypt(Nonce::from_slice(&blob[..12]), &blob[12..]).map_err(|e| e.to_string())
    }

    pub fn forget() {
        if let Ok(e) = entry() {
            let _ = e.delete_credential();
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)] // DPAPI is a Windows system call
mod imp {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    fn run(input: &[u8], protect: bool) -> Result<Vec<u8>, String> {
        let mut in_blob = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_ptr() as *mut u8 };
        let mut out_blob = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: the blobs point at live buffers for the duration of the call; the output is freed with LocalFree.
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &mut in_blob,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out_blob,
                )
            } else {
                CryptUnprotectData(
                    &mut in_blob,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out_blob,
                )
            }
        };
        if ok == 0 {
            return Err("DPAPI refused".into());
        }
        let out = unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec() };
        unsafe { LocalFree(out_blob.pbData as _) };
        Ok(out)
    }

    pub fn encrypt(plain: &[u8]) -> Result<Vec<u8>, String> {
        run(plain, true)
    }

    pub fn decrypt(blob: &[u8]) -> Result<Vec<u8>, String> {
        run(blob, false)
    }

    pub fn forget() {}
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    pub fn encrypt(_: &[u8]) -> Result<Vec<u8>, String> {
        Err("saving a sign-in needs macOS or Windows".into())
    }

    pub fn decrypt(_: &[u8]) -> Result<Vec<u8>, String> {
        Err("saving a sign-in needs macOS or Windows".into())
    }

    pub fn forget() {}
}

#[cfg(test)]
mod tests {
    // The Keychain/DPAPI round trip is checked on the release builders, where the OS store is available.
    #[test]
    #[cfg(windows)]
    fn dpapi_round_trip() {
        let blob = super::imp::encrypt(b"cookie=1").unwrap();
        assert_ne!(blob, b"cookie=1");
        assert_eq!(super::imp::decrypt(&blob).unwrap(), b"cookie=1");
    }
}
