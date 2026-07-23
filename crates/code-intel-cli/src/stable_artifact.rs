#[cfg(windows)]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FileId {
    volume: u64,
    file: u128,
}

#[derive(Debug)]
pub(crate) struct StableRead {
    pub(crate) bytes: Vec<u8>,
    pub(crate) id: FileId,
}

#[derive(Debug)]
pub(crate) enum StableReadError {
    TooLarge(String),
    Boundary(String),
    Identity(String),
    HostIo(String),
}

pub(crate) fn read_beneath(
    root: &Path,
    components: &[&str],
    max_bytes: u64,
) -> Result<StableRead, StableReadError> {
    read_beneath_with_hook(root, components, max_bytes, |_, _| Ok(()))
}

fn read_beneath_with_hook<F>(
    root: &Path,
    components: &[&str],
    max_bytes: u64,
    mut hook: F,
) -> Result<StableRead, StableReadError>
where
    F: FnMut(usize, &Path) -> Result<(), StableReadError>,
{
    if components.is_empty() {
        return Err(StableReadError::Boundary(
            "stable relative read requires at least one component".into(),
        ));
    }
    platform_read(root, components, max_bytes, &mut hook)
}

#[cfg(windows)]
fn platform_read<F>(
    root: &Path,
    components: &[&str],
    max_bytes: u64,
    hook: &mut F,
) -> Result<StableRead, StableReadError>
where
    F: FnMut(usize, &Path) -> Result<(), StableReadError>,
{
    let mut held = Vec::new();
    let root_handle = open_windows(root, true).map_err(|error| open_error(root, error, "root"))?;
    reject_kind(&root_handle, true, root)?;
    held.push((root.to_path_buf(), file_id(&root_handle)?, root_handle));
    let mut path = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        hook(index, &path)?;
        verify_held_windows(&held)?;
        path.push(component);
        let final_component = index + 1 == components.len();
        let opened = open_windows(&path, !final_component)
            .map_err(|error| component_open_error(&path, error))?;
        reject_kind(&opened, !final_component, &path)?;
        verify_held_windows(&held)?;
        if final_component {
            let id = file_id(&opened)?;
            let bytes = read_bounded(opened, max_bytes, &path)?;
            drop(held);
            return Ok(StableRead { bytes, id });
        }
        held.push((path.clone(), file_id(&opened)?, opened));
    }
    unreachable!()
}

#[cfg(windows)]
fn verify_held_windows(held: &[(PathBuf, FileId, File)]) -> Result<(), StableReadError> {
    for (path, expected, _) in held {
        let current =
            open_windows(path, true).map_err(|error| component_open_error(path, error))?;
        reject_kind(&current, true, path)?;
        if file_id(&current)? != *expected {
            return Err(StableReadError::Identity(format!(
                "artifact directory identity changed: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn open_windows(path: &Path, directory: bool) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    OpenOptions::new()
        .access_mode(if directory {
            FILE_READ_ATTRIBUTES
        } else {
            GENERIC_READ
        })
        .share_mode(1 | 2)
        .custom_flags(OPEN_REPARSE_POINT | if directory { BACKUP_SEMANTICS } else { 0 })
        .open(path)
}

#[cfg(target_os = "linux")]
fn platform_read<F>(
    root: &Path,
    components: &[&str],
    max_bytes: u64,
    hook: &mut F,
) -> Result<StableRead, StableReadError>
where
    F: FnMut(usize, &Path) -> Result<(), StableReadError>,
{
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    const O_CLOEXEC: i32 = 0x80000;
    const O_DIRECTORY: i32 = 0x10000;
    const O_NOFOLLOW: i32 = 0x20000;
    unsafe extern "C" {
        fn open(pathname: *const i8, flags: i32, ...) -> i32;
        fn openat(dirfd: i32, pathname: *const i8, flags: i32, ...) -> i32;
    }
    let root_c = CString::new(root.as_os_str().as_bytes())
        .map_err(|_| StableReadError::Boundary("artifact root contains NUL".to_string()))?;
    let root_fd = unsafe { open(root_c.as_ptr(), O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW) };
    if root_fd < 0 {
        return Err(open_error(root, std::io::Error::last_os_error(), "root"));
    }
    let root_file = unsafe { File::from_raw_fd(root_fd) };
    let mut held_paths = vec![(root.to_path_buf(), file_id(&root_file)?)];
    let mut held = vec![root_file];
    let mut path = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        hook(index, &path)?;
        verify_held_unix(&held_paths)?;
        let name = CString::new(component.as_bytes()).map_err(|_| {
            StableReadError::Boundary("artifact component contains NUL".to_string())
        })?;
        let final_component = index + 1 == components.len();
        let flags = O_CLOEXEC | O_NOFOLLOW | if final_component { 0 } else { O_DIRECTORY };
        let fd = unsafe { openat(held.last().unwrap().as_raw_fd(), name.as_ptr(), flags) };
        path.push(component);
        if fd < 0 {
            return Err(component_open_error(&path, std::io::Error::last_os_error()));
        }
        let opened = unsafe { File::from_raw_fd(fd) };
        reject_kind(&opened, !final_component, &path)?;
        verify_held_unix(&held_paths)?;
        if final_component {
            let id = file_id(&opened)?;
            let bytes = read_bounded(opened, max_bytes, &path)?;
            return Ok(StableRead { bytes, id });
        }
        held_paths.push((path.clone(), file_id(&opened)?));
        held.push(opened);
    }
    unreachable!()
}

#[cfg(target_os = "linux")]
fn verify_held_unix(held: &[(std::path::PathBuf, FileId)]) -> Result<(), StableReadError> {
    use std::os::unix::fs::MetadataExt;
    for (path, expected) in held {
        let metadata =
            fs::symlink_metadata(path).map_err(|error| component_open_error(path, error))?;
        if metadata.file_type().is_symlink()
            || (FileId {
                volume: metadata.dev(),
                file: metadata.ino() as u128,
            }) != *expected
        {
            return Err(StableReadError::Identity(format!(
                "artifact directory identity changed: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_read<F>(
    root: &Path,
    components: &[&str],
    max_bytes: u64,
    hook: &mut F,
) -> Result<StableRead, StableReadError>
where
    F: FnMut(usize, &Path) -> Result<(), StableReadError>,
{
    let mut held = Vec::new();
    let root_handle = File::open(root).map_err(|error| open_error(root, error, "root"))?;
    reject_kind(&root_handle, true, root)?;
    held.push(root_handle);
    let mut path = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        hook(index, &path)?;
        path.push(component);
        let final_component = index + 1 == components.len();
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| component_open_error(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(StableReadError::Boundary(format!(
                "artifact component is a symlink: {}",
                path.display()
            )));
        }
        let opened = File::open(&path).map_err(|error| component_open_error(&path, error))?;
        reject_kind(&opened, !final_component, &path)?;
        if final_component {
            let id = file_id(&opened)?;
            return Ok(StableRead {
                bytes: read_bounded(opened, max_bytes, &path)?,
                id,
            });
        }
        held.push(opened);
    }
    unreachable!()
}

fn reject_kind(handle: &File, directory: bool, path: &Path) -> Result<(), StableReadError> {
    let metadata = handle
        .metadata()
        .map_err(|error| StableReadError::HostIo(error.to_string()))?;
    if (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
        || reparse(&metadata)
    {
        return Err(StableReadError::Boundary(format!(
            "artifact component is not a plain {}: {}",
            if directory {
                "directory"
            } else {
                "regular file"
            },
            path.display()
        )));
    }
    Ok(())
}

fn open_error(path: &Path, error: std::io::Error, role: &str) -> StableReadError {
    if is_boundary_open_error(&error) {
        StableReadError::Boundary(format!(
            "stable artifact {role} is missing or not traversable without links: {}: {error}",
            path.display()
        ))
    } else {
        StableReadError::HostIo(format!(
            "open stable artifact {role} {}: {error}",
            path.display()
        ))
    }
}

fn component_open_error(path: &Path, error: std::io::Error) -> StableReadError {
    open_error(path, error, "component")
}

fn is_boundary_open_error(error: &std::io::Error) -> bool {
    if matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    ) {
        return true;
    }
    #[cfg(target_os = "linux")]
    if matches!(error.raw_os_error(), Some(20 | 40)) {
        return true;
    }
    false
}

fn read_bounded(handle: File, max_bytes: u64, path: &Path) -> Result<Vec<u8>, StableReadError> {
    if handle
        .metadata()
        .map_err(|error| StableReadError::HostIo(error.to_string()))?
        .len()
        > max_bytes
    {
        return Err(StableReadError::TooLarge(format!(
            "stable file exceeds {max_bytes} bytes"
        )));
    }
    let mut bytes = Vec::new();
    handle
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            StableReadError::HostIo(format!("read stable artifact {}: {error}", path.display()))
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(StableReadError::TooLarge(format!(
            "stable file exceeds {max_bytes} bytes"
        )));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn file_id(file: &File) -> Result<FileId, StableReadError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file
        .metadata()
        .map_err(|error| StableReadError::HostIo(error.to_string()))?;
    Ok(FileId {
        volume: metadata.dev(),
        file: metadata.ino() as u128,
    })
}

#[cfg(windows)]
fn file_id(file: &File) -> Result<FileId, StableReadError> {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Info {
        volume: u64,
        file_id: [u8; 16],
    }
    unsafe extern "system" {
        fn GetFileInformationByHandleEx(
            h: *mut c_void,
            class: i32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
    }
    let mut info = MaybeUninit::<Info>::uninit();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            18,
            info.as_mut_ptr().cast(),
            std::mem::size_of::<Info>() as u32,
        )
    };
    if ok == 0 {
        return Err(StableReadError::HostIo(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    let info = unsafe { info.assume_init() };
    Ok(FileId {
        volume: info.volume,
        file: u128::from_le_bytes(info.file_id),
    })
}

#[cfg(not(any(unix, windows)))]
fn file_id(file: &File) -> Result<FileId, StableReadError> {
    let metadata = file
        .metadata()
        .map_err(|error| StableReadError::HostIo(error.to_string()))?;
    Ok(FileId {
        volume: 0,
        file: metadata.len() as u128,
    })
}

#[cfg(unix)]
fn reparse(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    let kind = metadata.file_type();
    kind.is_symlink()
        || kind.is_block_device()
        || kind.is_char_device()
        || kind.is_fifo()
        || kind.is_socket()
}

#[cfg(windows)]
fn reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn held_parent_chain_rejects_intermediate_link_swap() {
        let root = std::env::temp_dir().join(format!(
            "code-intel-stable-chain-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let parent = root.join("parent");
        fs::create_dir_all(&parent).unwrap();
        fs::write(parent.join("payload"), b"owned").unwrap();
        let result = read_beneath_with_hook(&root, &["parent", "payload"], 32, |index, _| {
            if index == 1 {
                let renamed = root.join("renamed");
                if fs::rename(&parent, &renamed).is_ok() {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&renamed, &parent).unwrap();
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_dir(&renamed, &parent).unwrap();
                } else {
                    return Err(StableReadError::Identity(
                        "held parent blocked directory swap".to_string(),
                    ));
                }
            }
            Ok(())
        });
        assert!(result.is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_read_accepts_max_minus_one_and_max_but_rejects_max_plus_one() {
        let root = std::env::temp_dir().join(format!(
            "code-intel-stable-size-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        for (name, size, accepted) in [("low", 7, true), ("max", 8, true), ("high", 9, false)] {
            fs::write(root.join(name), vec![b'x'; size]).unwrap();
            let result = read_beneath(&root, &[name], 8);
            assert_eq!(result.is_ok(), accepted, "{name}: {result:?}");
            if !accepted {
                assert!(matches!(result, Err(StableReadError::TooLarge(_))));
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn root_link_is_a_boundary_not_a_traversal_authority() {
        let base = std::env::temp_dir().join(format!(
            "code-intel-stable-root-link-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let real = base.join("real");
        let link = base.join("link");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("payload"), b"x").unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&real, &link).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&real, &link).is_ok();
        if linked {
            assert!(matches!(
                read_beneath(&link, &["payload"], 8),
                Err(StableReadError::Boundary(_))
            ));
        }
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_root_intermediate_and_leaf_links_are_typed_boundaries() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!(
            "code-intel-linux-link-types-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let real = base.join("real");
        let nested = real.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("payload"), b"x").unwrap();
        symlink(&real, base.join("root-link")).unwrap();
        symlink(&nested, real.join("intermediate-link")).unwrap();
        symlink(nested.join("payload"), real.join("leaf-link")).unwrap();

        for result in [
            read_beneath(&base.join("root-link"), &["nested", "payload"], 8),
            read_beneath(&real, &["intermediate-link", "payload"], 8),
            read_beneath(&real, &["leaf-link"], 8),
        ] {
            assert!(matches!(result, Err(StableReadError::Boundary(_))));
        }
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn deny_share_lock_is_host_io() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!(
            "code-intel-stable-lock-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("payload");
        fs::write(&path, b"x").unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(matches!(
            read_beneath(&root, &["payload"], 8),
            Err(StableReadError::HostIo(_))
        ));
        drop(lock);
        let _ = fs::remove_dir_all(root);
    }
}
