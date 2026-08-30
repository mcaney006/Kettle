use super::InfrastructureError;
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub fn validated_askpass_helper(
    current_executable: &Path,
) -> Result<Option<PathBuf>, InfrastructureError> {
    let Some(directory) = current_executable.parent() else {
        return Ok(None);
    };
    let helper = directory.join("kettle-askpass");
    let metadata = match std::fs::symlink_metadata(&helper) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(InfrastructureError::filesystem(&helper, source)),
    };
    let executable_metadata = std::fs::metadata(current_executable)
        .map_err(|source| InfrastructureError::filesystem(current_executable, source))?;
    if !metadata.file_type().is_file() {
        return Err(InfrastructureError::PrivilegeHelper(
            "helper is not a regular file".to_owned(),
        ));
    }
    if metadata.uid() != executable_metadata.uid() {
        return Err(InfrastructureError::PrivilegeHelper(
            "helper owner differs from the application executable".to_owned(),
        ));
    }
    if metadata.mode() & 0o022 != 0 || metadata.mode() & 0o111 == 0 {
        return Err(InfrastructureError::PrivilegeHelper(
            "helper must be executable and not group/world writable".to_owned(),
        ));
    }
    Ok(Some(helper))
}
