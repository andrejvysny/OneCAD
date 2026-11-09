use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

pub fn rewrite(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(bytes)
}

pub fn sha(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_existing_file_and_hardlink_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::write(&source, b"fixture").unwrap();
        let alias = dir.path().join("hardlink");
        std::fs::hard_link(&source, &alias).unwrap();
        for path in [&source, &alias] {
            assert_eq!(
                create(path).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(std::fs::read(&source).unwrap(), b"fixture");
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_and_dangling_symlink_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::write(&source, b"fixture").unwrap();
        for (name, target) in [
            ("alias", source.as_path()),
            ("dangling", Path::new("missing")),
        ] {
            let alias = dir.path().join(name);
            std::os::unix::fs::symlink(target, &alias).unwrap();
            assert_eq!(
                create(&alias).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(std::fs::read(&source).unwrap(), b"fixture");
        }
        assert!(!dir.path().join("missing").exists());
    }

    #[test]
    fn rewrites_only_owned_file_and_streams_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report");
        let mut file = create(&path).unwrap();
        rewrite(&mut file, &vec![b'x'; 65537]).unwrap();
        assert_eq!(
            sha(&path).unwrap(),
            format!("{:x}", Sha256::digest(vec![b'x'; 65537]))
        );
        rewrite(&mut file, b"short").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"short");
    }
}
