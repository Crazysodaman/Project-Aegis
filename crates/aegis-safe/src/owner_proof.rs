use std::fs;
use std::path::{Path, PathBuf};

use getrandom::getrandom;

use crate::SafeError;

#[cfg(windows)]
pub const OWNER_PROOF_KIND: &str = "dpapi-current-user";
#[cfg(not(windows))]
pub const OWNER_PROOF_KIND: &str = "unix-file-dev";

const TOKEN_BYTES: usize = 32;

#[derive(Debug, Clone)]
pub struct LocalOwnerProof {
    path: PathBuf,
}

impl LocalOwnerProof {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn initialize(&self) -> Result<String, SafeError> {
        if self.path.exists() {
            return Err(SafeError::OwnerAlreadyInitialized);
        }
        let token = random_token()?;
        let protected = protect(token.as_bytes())?;
        write_atomic(&self.path, &protected)?;
        Ok(token)
    }

    pub fn verify(&self, candidate: &str) -> Result<bool, SafeError> {
        let expected = self.read_token_bytes()?;
        let candidate = candidate.as_bytes();
        Ok(constant_time_eq(&expected, candidate))
    }

    pub fn load_token(&self) -> Result<String, SafeError> {
        let bytes = self.read_token_bytes()?;
        String::from_utf8(bytes).map_err(|_| SafeError::OwnerProofInvalid)
    }

    fn read_token_bytes(&self) -> Result<Vec<u8>, SafeError> {
        if !self.path.is_file() {
            return Err(SafeError::OwnerNotInitialized);
        }
        let protected = fs::read(&self.path)?;
        unprotect(&protected)
    }
}

fn random_token() -> Result<String, SafeError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom(&mut bytes).map_err(|_| SafeError::OwnerProofInvalid)?;
    Ok(hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(a ^ b);
    }
    difference == 0
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), SafeError> {
    let nonce = {
        let mut bytes = [0u8; 8];
        getrandom(&mut bytes).map_err(|_| SafeError::OwnerProofInvalid)?;
        hex_encode(&bytes)
    };
    let temporary = path.with_extension(format!("tmp-{nonce}"));
    fs::write(&temporary, data)?;
    set_private_permissions(&temporary)?;
    fs::rename(&temporary, path)?;
    set_private_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), SafeError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), SafeError> {
    Ok(())
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Result<Vec<u8>, SafeError> {
    windows_dpapi::protect(data)
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Result<Vec<u8>, SafeError> {
    windows_dpapi::unprotect(data)
}

#[cfg(not(windows))]
fn protect(data: &[u8]) -> Result<Vec<u8>, SafeError> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> Result<Vec<u8>, SafeError> {
    // The Unix development fallback stores the token in a 0600 file.
    Ok(data.to_vec())
}

#[cfg(windows)]
mod windows_dpapi {
    #![allow(unsafe_code)]

    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    use crate::SafeError;

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, SafeError> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: input/output blobs point to valid memory for the call. The
        // returned output is freed with LocalFree before returning.
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 || output.pbData.is_null() {
            return Err(dpapi_error());
        }
        // SAFETY: DPAPI returned a buffer of output.cbData bytes.
        let result =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        // SAFETY: LocalFree releases the buffer allocated by DPAPI.
        unsafe {
            LocalFree(output.pbData as *mut c_void);
        }
        Ok(result)
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, SafeError> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let mut description: *mut u16 = std::ptr::null_mut();
        // SAFETY: input/output blobs and description pointer are valid for the
        // call. Output and description are released below.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                &mut description,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if !description.is_null() {
            // SAFETY: DPAPI allocated the description with LocalAlloc.
            unsafe {
                LocalFree(description as *mut c_void);
            }
        }
        if ok == 0 || output.pbData.is_null() {
            return Err(dpapi_error());
        }
        // SAFETY: DPAPI returned a buffer of output.cbData bytes.
        let result =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        // SAFETY: LocalFree releases the buffer allocated by DPAPI.
        unsafe {
            LocalFree(output.pbData as *mut c_void);
        }
        Ok(result)
    }

    fn dpapi_error() -> SafeError {
        SafeError::Io(std::io::Error::other("DPAPI operation failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_verifies_and_wrong_token_fails() {
        let temp = tempfile::tempdir().expect("tempdir");
        let proof = LocalOwnerProof::new(temp.path().join("owner.proof"));
        let token = proof.initialize().expect("initialize");
        assert!(proof.verify(&token).expect("verify"));
        assert!(!proof.verify("not-the-token").expect("verify wrong"));
        assert_eq!(proof.load_token().expect("load"), token);
    }

    #[test]
    fn second_initialization_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let proof = LocalOwnerProof::new(temp.path().join("owner.proof"));
        proof.initialize().expect("initialize");
        assert!(matches!(
            proof.initialize(),
            Err(SafeError::OwnerAlreadyInitialized)
        ));
    }
}
