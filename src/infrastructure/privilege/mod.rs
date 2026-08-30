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
    let directory_metadata = std::fs::symlink_metadata(directory)
        .map_err(|source| InfrastructureError::filesystem(directory, source))?;
    if !directory_metadata.is_dir()
        || directory_metadata.uid() != executable_metadata.uid()
        || directory_metadata.mode() & 0o022 != 0
    {
        return Err(InfrastructureError::PrivilegeHelper(
            "helper directory has unsafe ownership or permissions".to_owned(),
        ));
    }
    let canonical = helper
        .canonicalize()
        .map_err(|source| InfrastructureError::filesystem(&helper, source))?;
    if canonical.parent() != directory.canonicalize().ok().as_deref() {
        return Err(InfrastructureError::PrivilegeHelper(
            "helper must not resolve outside the application directory".to_owned(),
        ));
    }
    Ok(Some(canonical))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn helper_must_stay_in_a_non_writable_executable_directory() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("MacOS");
        let executable = directory.join("kettle");
        let helper = directory.join("kettle-askpass");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(&executable, "app").unwrap();
        std::fs::write(&helper, "helper").unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            validated_askpass_helper(&executable).unwrap(),
            Some(helper.canonicalize().unwrap())
        );

        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validated_askpass_helper(&executable).is_err());
    }
}
