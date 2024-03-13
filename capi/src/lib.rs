use std::ffi::CString;
use std::io::BufReader;
use std::mem::size_of_val;
use std::os::unix::ffi::OsStrExt;
use std::ptr::null_mut;

use errno::{set_errno, Errno};
use libc::{c_char, c_int, c_void, size_t};
use pure::parse_jeheap;
use tempfile::NamedTempFile;

pub const JP_SUCCESS: c_int = 0;
pub const JP_FAILURE: c_int = -1;

#[link(name = "jemalloc")]
extern "C" {
    // int mallctl(const char *name, void *oldp, size_t *oldlenp, void *newp, size_t newlen);
    fn mallctl(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut size_t,
        newp: *mut c_void,
        newlen: size_t,
    ) -> c_int;
}

enum Error {
    Io(std::io::Error),
    Mallctl(c_int),
    Anyhow(anyhow::Error),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Self::Anyhow(e)
    }
}

fn dump_pprof_inner() -> Result<Vec<u8>, Error> {
    let f = NamedTempFile::new()?;
    let path = CString::new(f.path().as_os_str().as_bytes().to_vec()).unwrap();
    // SAFETY: "prof.dump" is documented as being writable and taking a C string as input:
    // http://jemalloc.net/jemalloc.3.html#prof.dump
    let pp = (&mut path.as_ptr()) as *mut _ as *mut _;
    let ret = unsafe {
        mallctl(
            b"prof.dump\0" as *const _ as *const c_char,
            null_mut(),
            null_mut(),
            pp,
            size_of_val(&pp),
        )        
    };
    if ret != 0 {
        return Err(Error::Mallctl(ret));
    }

    let dump_reader = BufReader::new(f);
    let profile = parse_jeheap(dump_reader)?;
    let pprof = profile.to_pprof(("inuse_space", "bytes"), ("space", "bytes"), None);
    Ok(pprof)

}

#[no_mangle]
pub unsafe extern "C" fn dump_pprof(buf_out: *mut *mut u8, n_out: *mut size_t) -> c_int {
    let buf = match dump_pprof_inner() {
        Ok(buf) => buf,
        Err(Error::Io(e)) if e.raw_os_error().is_some() => {
            set_errno(Errno(e.raw_os_error().unwrap()));
            return JP_FAILURE;
        },
        Err(Error::Mallctl(i)) => {
            set_errno(Errno(i));
            return JP_FAILURE;
        }
        // TODO - maybe some of these can have errnos
        Err(_) => {
            return JP_FAILURE;
        }
    };
    let len: size_t = buf.len().try_into().expect("absurd length");
    // leak is ok, consumer is responsible for freeing
    let p = buf.leak().as_mut_ptr();
    unsafe {
        std::ptr::write(buf_out, p);
        std::ptr::write(n_out, len);
    }
    JP_SUCCESS
}
