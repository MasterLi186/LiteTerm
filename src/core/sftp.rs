use ssh2::Sftp;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

/// Metadata for a remote file or directory entry.
#[derive(Debug, Clone)]
pub struct SftpEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
    pub permissions: u32,
}

/// Wrapper around ssh2::Sftp providing file operations.
pub struct SftpOps<'a> {
    sftp: &'a Sftp,
}

impl<'a> SftpOps<'a> {
    /// Create a new SftpOps wrapping an ssh2 Sftp session.
    pub fn new(sftp: &'a Sftp) -> Self {
        Self { sftp }
    }

    /// List entries in a remote directory.
    pub fn list_dir(&self, path: &str) -> io::Result<Vec<SftpEntry>> {
        let dir = self
            .sftp
            .readdir(Path::new(path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let entries = dir
            .into_iter()
            .filter_map(|(pathbuf, stat)| {
                let name = pathbuf.file_name()?.to_string_lossy().into_owned();
                Some(SftpEntry {
                    name,
                    is_dir: stat.is_dir(),
                    size: stat.size.unwrap_or(0),
                    mtime: stat.mtime.unwrap_or(0),
                    permissions: stat.perm.unwrap_or(0o644),
                })
            })
            .collect();

        Ok(entries)
    }

    /// Download a remote file to a local path, calling progress_cb with bytes read so far.
    pub fn download<F>(
        &self,
        remote_path: &str,
        local_path: &str,
        mut progress_cb: F,
    ) -> io::Result<u64>
    where
        F: FnMut(u64),
    {
        let mut remote_file = self
            .sftp
            .open(Path::new(remote_path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let mut local_file = fs::File::create(local_path)?;
        let mut buf = [0u8; 32768];
        let mut total: u64 = 0;

        loop {
            let n = remote_file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            local_file.write_all(&buf[..n])?;
            total += n as u64;
            progress_cb(total);
        }

        Ok(total)
    }

    /// Upload a local file to a remote path, calling progress_cb with bytes written so far.
    pub fn upload<F>(
        &self,
        local_path: &str,
        remote_path: &str,
        mut progress_cb: F,
    ) -> io::Result<u64>
    where
        F: FnMut(u64),
    {
        let mut local_file = fs::File::open(local_path)?;
        let mut remote_file = self
            .sftp
            .create(Path::new(remote_path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let mut buf = [0u8; 32768];
        let mut total: u64 = 0;

        loop {
            let n = local_file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            remote_file.write_all(&buf[..n])?;
            total += n as u64;
            progress_cb(total);
        }

        Ok(total)
    }

    /// Create a remote directory with the given permissions mode.
    pub fn mkdir(&self, path: &str, mode: i32) -> io::Result<()> {
        self.sftp
            .mkdir(Path::new(path), mode)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    /// Remove a remote file.
    pub fn remove_file(&self, path: &str) -> io::Result<()> {
        self.sftp
            .unlink(Path::new(path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    /// Remove a remote directory.
    pub fn remove_dir(&self, path: &str) -> io::Result<()> {
        self.sftp
            .rmdir(Path::new(path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    /// Rename (move) a remote file or directory.
    pub fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        self.sftp
            .rename(Path::new(from), Path::new(to), None)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    /// Get metadata for a remote path.
    pub fn stat(&self, path: &str) -> io::Result<SftpEntry> {
        let stat = self
            .sftp
            .stat(Path::new(path))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        Ok(SftpEntry {
            name,
            is_dir: stat.is_dir(),
            size: stat.size.unwrap_or(0),
            mtime: stat.mtime.unwrap_or(0),
            permissions: stat.perm.unwrap_or(0o644),
        })
    }
}
