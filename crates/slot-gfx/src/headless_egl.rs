//! Mesa's EGL with no window, so the render tests run on Linux as they do on macOS.
use crate::surface::{GfxError, Surface, OUT_H, OUT_W};
use libloading::Library;
use std::ffi::{c_void, CString};

type Ptr = *mut c_void;
pub struct HeadlessSurface {
    // Never unloaded: Mesa keeps threads that would run unloaded code at exit.
    egl: &'static Library,
    gles: &'static Library,
    display: Ptr,
    surface: Ptr,
    context: Ptr,
}

impl HeadlessSurface {
    pub fn new() -> Result<Self, GfxError> {
        std::env::set_var("EGL_PLATFORM", "surfaceless");
        let err = |m: &str| GfxError::Context(m.into());
        unsafe {
            let egl: &'static Library = Box::leak(Box::new(
                Library::new("libEGL.so.1").map_err(|e| err(&e.to_string()))?,
            ));
            let gles: &'static Library = Box::leak(Box::new(
                Library::new("libGLESv2.so.2").map_err(|e| err(&e.to_string()))?,
            ));
            let get_display: libloading::Symbol<unsafe extern "C" fn(Ptr) -> Ptr> =
                egl.get(b"eglGetDisplay").unwrap();
            let initialize: libloading::Symbol<
                unsafe extern "C" fn(Ptr, *mut i32, *mut i32) -> u32,
            > = egl.get(b"eglInitialize").unwrap();
            let bind_api: libloading::Symbol<unsafe extern "C" fn(u32) -> u32> =
                egl.get(b"eglBindAPI").unwrap();
            let choose: libloading::Symbol<
                unsafe extern "C" fn(Ptr, *const i32, *mut Ptr, i32, *mut i32) -> u32,
            > = egl.get(b"eglChooseConfig").unwrap();
            let pbuffer: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, *const i32) -> Ptr> =
                egl.get(b"eglCreatePbufferSurface").unwrap();
            let create_context: libloading::Symbol<
                unsafe extern "C" fn(Ptr, Ptr, Ptr, *const i32) -> Ptr,
            > = egl.get(b"eglCreateContext").unwrap();
            let make_current: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> u32> =
                egl.get(b"eglMakeCurrent").unwrap();
            let display = get_display(std::ptr::null_mut());
            if display.is_null()
                || initialize(display, std::ptr::null_mut(), std::ptr::null_mut()) == 0
            {
                return Err(err("egl display"));
            }
            bind_api(0x30A0);
            let attrs = [
                0x3033, 0x0001, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3040, 0x0004, 0x3038,
            ];
            let (mut config, mut n) = (std::ptr::null_mut(), 0);
            if choose(display, attrs.as_ptr(), &mut config, 1, &mut n) == 0 || n == 0 {
                return Err(err("egl config"));
            }
            let size = [0x3057, OUT_W as i32, 0x3056, OUT_H as i32, 0x3038];
            let surface = pbuffer(display, config, size.as_ptr());
            let ctx_attrs = [0x3098, 2, 0x3038];
            let context = create_context(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr());
            if surface.is_null()
                || context.is_null()
                || make_current(display, surface, surface, context) == 0
            {
                return Err(err("egl surface/context"));
            }
            let s = HeadlessSurface {
                egl,
                gles,
                display,
                surface,
                context,
            };
            Ok(s)
        }
    }
}

impl Surface for HeadlessSurface {
    fn make_current(&mut self) -> Result<(), GfxError> {
        let f: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> u32> =
            unsafe { self.egl.get(b"eglMakeCurrent").unwrap() };
        unsafe { f(self.display, self.surface, self.surface, self.context) };
        Ok(())
    }
    fn window_size(&self) -> (u32, u32) {
        (OUT_W, OUT_H)
    }
    fn swap(&mut self) -> Result<(), GfxError> {
        Ok(())
    }
    fn proc_address(&self, name: &str) -> *const c_void {
        let Ok(c) = CString::new(name) else {
            return std::ptr::null();
        };
        unsafe {
            if let Ok(f) = self.gles.get::<*const c_void>(c.as_bytes_with_nul()) {
                return *f;
            }
            let gpa: libloading::Symbol<unsafe extern "C" fn(*const i8) -> *const c_void> =
                self.egl.get(b"eglGetProcAddress").unwrap();
            gpa(c.as_ptr())
        }
    }
}

impl Drop for HeadlessSurface {
    fn drop(&mut self) {
        type Current = unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> u32;
        type Destroy = unsafe extern "C" fn(Ptr, Ptr) -> u32;
        unsafe {
            let none = std::ptr::null_mut();
            if let Ok(f) = self.egl.get::<Current>(b"eglMakeCurrent") {
                f(self.display, none, none, none);
            }
            if let Ok(f) = self.egl.get::<Destroy>(b"eglDestroyContext") {
                f(self.display, self.context);
            }
            if let Ok(f) = self.egl.get::<Destroy>(b"eglDestroySurface") {
                f(self.display, self.surface);
            }
        }
    }
}
