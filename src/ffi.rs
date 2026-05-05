//! C ABI surface called from Unity's TPSheetPostprocessor via P/Invoke.
//! See CLAUDE.md "C# ↔ Rust contract" for the canonical spec; mirror it
//! exactly when changing this file.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use crate::pipeline;

/// Bump on every breaking change to the FFI struct layout or function set.
/// C# asserts equality at load time and refuses to call mismatched libs.
const ABI_VERSION: u32 = 1;

#[repr(C)]
pub struct GenerateInputs {
    pub tpsheet_path: *const c_char,
    pub tps_path: *const c_char,
    pub atlas_png_path: *const c_char,
    pub sprite_dir: *const c_char,
    pub prefix: *const c_char,
    pub ppu: f32,
}

/// Owns its allocations; freed via [`free_output`].
#[repr(C)]
pub struct GenerateOutput {
    pub written_paths: *const *const c_char,
    pub written_len: usize,
    pub deleted_paths: *const *const c_char,
    pub deleted_len: usize,
    /// Opaque pointer to the Rust-side arena. C# treats it as void*.
    /// Do not deref; only pass to free_output.
    _arena: *mut OutputArena,
}

#[repr(C)]
pub struct ErrorOut {
    pub code: i32,
    pub message: *const c_char,
}

/// Backing storage for [`GenerateOutput`]. Held alive in Rust until C# calls
/// `free_output`. Layout is private; only the pointer is shared with C#.
struct OutputArena {
    // Owned C strings, kept alive until the arena drops.
    _strings: Vec<CString>,
    // Pointer arrays the FFI exposes (point into _strings).
    written_ptrs: Vec<*const c_char>,
    deleted_ptrs: Vec<*const c_char>,
}

#[unsafe(no_mangle)]
pub extern "C" fn abi_version() -> u32 {
    ABI_VERSION
}

/// Run the pipeline. Returns 0 on success, non-zero on error (with `err`
/// populated). On success, `out` is populated and must be freed via
/// [`free_output`]. On error, `err` must be freed via [`free_error`].
///
/// # Safety
///
/// All pointer fields of `*input` must be valid null-terminated UTF-8 C
/// strings, or null in which case the function returns an error. `out` and
/// `err` must be valid writable pointers to uninitialized structs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn generate(
    input: *const GenerateInputs,
    out: *mut GenerateOutput,
    err: *mut ErrorOut,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| unsafe { generate_inner(input, out) }));
    match result {
        Ok(Ok(())) => {
            unsafe {
                if !err.is_null() {
                    err.write(ErrorOut {
                        code: 0,
                        message: std::ptr::null(),
                    });
                }
            }
            0
        }
        Ok(Err(message)) => unsafe {
            write_error(err, 1, &message);
            1
        },
        Err(panic_payload) => {
            let msg = panic_to_string(panic_payload);
            unsafe { write_error(err, 2, &msg) };
            2
        }
    }
}

/// # Safety
///
/// `out` must have been populated by a prior successful call to [`generate`].
/// After this call the inner pointers are invalid; do not dereference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_output(out: *mut GenerateOutput) {
    if out.is_null() {
        return;
    }
    let arena_ptr = unsafe { (*out)._arena };
    if !arena_ptr.is_null() {
        unsafe {
            drop(Box::from_raw(arena_ptr));
        }
    }
    unsafe {
        out.write(GenerateOutput {
            written_paths: std::ptr::null(),
            written_len: 0,
            deleted_paths: std::ptr::null(),
            deleted_len: 0,
            _arena: std::ptr::null_mut(),
        });
    }
}

/// # Safety
///
/// `err` must have been populated by a prior call to [`generate`] that
/// returned non-zero. The `message` pointer is invalid after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_error(err: *mut ErrorOut) {
    if err.is_null() {
        return;
    }
    let msg = unsafe { (*err).message };
    if !msg.is_null() {
        unsafe {
            drop(CString::from_raw(msg as *mut c_char));
        }
    }
    unsafe {
        err.write(ErrorOut {
            code: 0,
            message: std::ptr::null(),
        });
    }
}

// ---- internals -----------------------------------------------------------

unsafe fn generate_inner(
    input: *const GenerateInputs,
    out: *mut GenerateOutput,
) -> Result<(), String> {
    if input.is_null() {
        return Err("input pointer is null".into());
    }
    if out.is_null() {
        return Err("output pointer is null".into());
    }
    let inp = unsafe { &*input };
    let tpsheet = unsafe { cstr_to_path(inp.tpsheet_path, "tpsheet_path") }?;
    let tps = unsafe { cstr_to_path(inp.tps_path, "tps_path") }?;
    let atlas_png = unsafe { cstr_to_path(inp.atlas_png_path, "atlas_png_path") }?;
    let sprite_dir = unsafe { cstr_to_path(inp.sprite_dir, "sprite_dir") }?;
    let prefix = unsafe { cstr_to_str(inp.prefix, "prefix") }?;

    let inputs = pipeline::GenerateInputs {
        tpsheet_path: &tpsheet,
        tps_path: &tps,
        atlas_png_path: &atlas_png,
        sprite_dir: &sprite_dir,
        prefix,
        ppu: inp.ppu,
    };

    let result = pipeline::generate(&inputs).map_err(|e| format!("{e}"))?;
    unsafe { write_output(out, result) };
    Ok(())
}

fn build_arena(
    written: Vec<PathBuf>,
    deleted: Vec<PathBuf>,
) -> Result<Box<OutputArena>, String> {
    let mut strings: Vec<CString> = Vec::with_capacity(written.len() + deleted.len());
    let mut written_ptrs: Vec<*const c_char> = Vec::with_capacity(written.len());
    let mut deleted_ptrs: Vec<*const c_char> = Vec::with_capacity(deleted.len());
    for p in written.iter().chain(deleted.iter()) {
        let s = p
            .to_str()
            .ok_or_else(|| format!("non-UTF-8 path: {p:?}"))?;
        strings.push(CString::new(s).map_err(|e| format!("path contains NUL: {e}"))?);
    }
    let (w, d) = strings.split_at(written.len());
    for s in w {
        written_ptrs.push(s.as_ptr());
    }
    for s in d {
        deleted_ptrs.push(s.as_ptr());
    }
    Ok(Box::new(OutputArena {
        _strings: strings,
        written_ptrs,
        deleted_ptrs,
    }))
}

unsafe fn write_output(out: *mut GenerateOutput, result: pipeline::GenerateOutput) {
    let arena_result = build_arena(result.written_paths, result.deleted_paths);
    match arena_result {
        Ok(arena) => {
            let written_paths = arena.written_ptrs.as_ptr();
            let written_len = arena.written_ptrs.len();
            let deleted_paths = arena.deleted_ptrs.as_ptr();
            let deleted_len = arena.deleted_ptrs.len();
            let arena_ptr = Box::into_raw(arena);
            unsafe {
                out.write(GenerateOutput {
                    written_paths,
                    written_len,
                    deleted_paths,
                    deleted_len,
                    _arena: arena_ptr,
                });
            }
        }
        Err(_) => {
            // Empty output; caller treats len=0 as no-op. Errors at this
            // stage are rare (path-encoding edge cases) and not worth
            // surfacing through the success channel.
            unsafe {
                out.write(GenerateOutput {
                    written_paths: std::ptr::null(),
                    written_len: 0,
                    deleted_paths: std::ptr::null(),
                    deleted_len: 0,
                    _arena: std::ptr::null_mut(),
                });
            }
        }
    }
}

unsafe fn write_error(err: *mut ErrorOut, code: i32, message: &str) {
    if err.is_null() {
        return;
    }
    let cstr = match CString::new(message) {
        Ok(c) => c,
        Err(_) => CString::new("<error message contained NUL>").unwrap(),
    };
    let ptr = cstr.into_raw();
    unsafe {
        err.write(ErrorOut {
            code,
            message: ptr as *const c_char,
        });
    }
}

unsafe fn cstr_to_path(p: *const c_char, label: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(unsafe { cstr_to_str(p, label) }?))
}

unsafe fn cstr_to_str<'a>(p: *const c_char, label: &str) -> Result<&'a str, String> {
    if p.is_null() {
        return Err(format!("{label} is null"));
    }
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .map_err(|e| format!("{label} not UTF-8: {e}"))
}

fn panic_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return format!("panic: {s}");
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return format!("panic: {s}");
    }
    "panic: <opaque>".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_inputs_return_error_via_err_struct() {
        let mut out = GenerateOutput {
            written_paths: std::ptr::null(),
            written_len: 0,
            deleted_paths: std::ptr::null(),
            deleted_len: 0,
            _arena: std::ptr::null_mut(),
        };
        let mut err = ErrorOut {
            code: 0,
            message: std::ptr::null(),
        };
        let rc = unsafe { generate(std::ptr::null(), &mut out, &mut err) };
        assert_eq!(rc, 1);
        assert_eq!(err.code, 1);
        assert!(!err.message.is_null());
        unsafe { free_error(&mut err) };
    }

    #[test]
    fn abi_version_returns_constant() {
        assert_eq!(abi_version(), ABI_VERSION);
    }
}
