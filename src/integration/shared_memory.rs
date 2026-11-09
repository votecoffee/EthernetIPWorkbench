use super::{Feed, MAX_ASSEMBLY_BYTES};
use std::{io, ptr, sync::atomic::Ordering::Relaxed};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
        SetLastError, WAIT_ABANDONED, WAIT_OBJECT_0,
    },
    System::{
        Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
            PAGE_READWRITE, UnmapViewOfFile,
        },
        Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
};

const HEADER: usize = 64;
const SIZE: usize = HEADER + MAX_ASSEMBLY_BYTES;

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub(super) struct Mapping {
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    _file: Handle,
    lock: Handle,
    published: Option<(u64, u64)>,
}

// The mapping and handles are owned by this value and transferred to one
// dispatcher thread. Each view access is serialized by the named mutex.
unsafe impl Send for Mapping {}

impl Mapping {
    pub(super) fn new(name: &str) -> io::Result<Self> {
        let prefix = format!("Local\\EthernetIPWorkbench.{name}");
        let wide = |suffix: &str| {
            format!("{prefix}.{suffix}")
                .encode_utf16()
                .chain([0])
                .collect::<Vec<_>>()
        };
        let lock =
            create_exclusive(|| unsafe { CreateMutexW(ptr::null(), 0, wide("lock").as_ptr()) })?;
        let file = create_exclusive(|| unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                ptr::null(),
                PAGE_READWRITE,
                0,
                SIZE as u32,
                wide("data").as_ptr(),
            )
        })?;
        let view = unsafe { MapViewOfFile(file.0, FILE_MAP_WRITE, 0, 0, SIZE) };
        if view.Value.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            view,
            _file: file,
            lock,
            published: None,
        })
    }

    pub(super) fn update(&mut self, feed: &Feed) {
        self.write(feed, feed.active_generation.load(Relaxed), 0);
    }

    pub(super) fn close(&mut self, feed: &Feed) {
        self.write(feed, 0, 100);
    }

    fn write(&mut self, feed: &Feed, active: u64, wait_ms: u32) {
        let frame = feed.latest();
        let index = frame.map_or(0, |frame| frame.index);
        if self.published == Some((index, active)) {
            return;
        }
        let result = unsafe { WaitForSingleObject(self.lock.0, wait_ms) };
        if !matches!(result, WAIT_OBJECT_0 | WAIT_ABANDONED) {
            feed.memory_busy.fetch_add(1, Relaxed);
            return;
        }
        let mut bytes = [0u8; SIZE];
        bytes[..8].copy_from_slice(b"EIPWB001");
        bytes[8..16].copy_from_slice(&feed.started_utc_us.to_le_bytes());
        bytes[16..24].copy_from_slice(&index.to_le_bytes());
        bytes[32..40].copy_from_slice(&active.to_le_bytes());
        bytes[54..56].copy_from_slice(&feed.settings.input_instance.to_le_bytes());
        if let Some(frame) = frame {
            bytes[24..32].copy_from_slice(&frame.generation.to_le_bytes());
            bytes[40..48].copy_from_slice(&frame.elapsed_us.to_le_bytes());
            bytes[48..52].copy_from_slice(&frame.packet_sequence.to_le_bytes());
            bytes[52..54].copy_from_slice(&frame.cip_sequence.to_le_bytes());
            bytes[56..58].copy_from_slice(&frame.size.to_le_bytes());
            bytes[HEADER..HEADER + usize::from(frame.size)].copy_from_slice(frame.bytes());
        }
        // The page view is exactly SIZE bytes and the mutex is owned here.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), self.view.Value.cast(), SIZE);
            ReleaseMutex(self.lock.0);
        }
        self.published = Some((index, active));
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe { UnmapViewOfFile(self.view) };
    }
}

fn create_exclusive(create: impl FnOnce() -> HANDLE) -> io::Result<Handle> {
    unsafe { SetLastError(0) };
    let handle = create();
    let code = unsafe { GetLastError() };
    if handle.is_null() {
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    let handle = Handle(handle);
    if code == ERROR_ALREADY_EXISTS {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "mapping or mutex name is already in use",
        ));
    }
    Ok(handle)
}
