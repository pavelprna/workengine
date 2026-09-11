use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use workengine_application::AppError;

fn no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "control-plane path is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    no_follow(&mut options);
    let mut file = options.open(path).map_err(AppError::worker)?;
    if !file.metadata().map_err(AppError::worker)?.is_file() {
        return Err(AppError::worker("control-plane path is not a regular file"));
    }
    file.write_all(bytes).map_err(AppError::worker)?;
    file.sync_all().map_err(AppError::worker)
}

pub(crate) fn create_private(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    no_follow(&mut options);
    let mut file: File = options.open(path).map_err(AppError::worker)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(AppError::worker)?;
    }
    file.write_all(bytes).map_err(AppError::worker)?;
    file.sync_all().map_err(AppError::worker)
}
