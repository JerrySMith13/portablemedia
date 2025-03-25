use std::{
    collections::HashMap,
    fs::{self, read_dir},
    io::{self, Error, ErrorKind},
    num::NonZeroUsize,
    os::unix::fs::MetadataExt,
    sync::{Arc, Mutex},
};
type DirMap = HashMap<String, Arc<FileNode>>;
use lru::LruCache;
use tokio::io::AsyncReadExt;

use crate::log::{self, log_err};

pub struct FileNode {
    pub name: String,
    pub size: u64,
    pub children: Option<DirMap>,
}

impl FileNode {
    fn build_from_path(path: &str) -> Result<FileNode, Error> {
        let file = fs::File::open(path)?;
        let metadata = file.metadata()?;

        let mut size: u64 = 0;
        let name = match path.split("/").last() {
            Some(s) => s.to_string(),
            None => {
                return Err(Error::new(
                    ErrorKind::Other,
                    format!("Error in trying to assign name to file {}", path),
                ))
            }
        };
        let children: Option<DirMap>;

        if metadata.is_symlink() {
            return Err(Error::new(
                ErrorKind::Other,
                format!(
                    "Error: file {} is a symlink (symlinks are not currently supported)",
                    path
                ),
            ));
        }
        if metadata.is_file() {
            size = metadata.size();
            children = None;
        } else {
            //Safe unwrap because we know for a fact it's a directory, nothing about the file state can change
            let directory = read_dir(path).unwrap();
            let mut children_map: DirMap = HashMap::with_capacity(directory.size_hint().0);
            for file in directory {
                if file.is_err() {
                    log_err(
                        format!(
                            "Error in reading a file in directory {}, skipping file",
                            path
                        )
                        .as_str(),
                        log::LogPriority::Middle,
                    );
                    continue;
                }
                let file_name = file.unwrap().file_name().into_string();
                if file_name.is_err() {
                    log_err(
                        format!(
                            "Error: filename {} in directory {} not valid unicode, skipping file",
                            file_name.unwrap_err().to_string_lossy().into_owned(),
                            path
                        )
                        .as_str(),
                        log::LogPriority::Middle,
                    );
                    continue;
                }
                let file_name = file_name.unwrap();
                children_map.insert(
                    file_name.clone(),
                    Arc::new(FileNode::build_from_path(
                        format!("{}/{}", path, file_name).as_str(),
                    )?),
                );
            }
            children = Some(children_map);
        }
        return Ok(FileNode {
            name,
            size,
            children,
        });
    }
}

pub struct FileMap {
    FULL_ROOT_PATH: String,
    head: Arc<FileNode>,
    lru: Arc<Mutex<LruCache<String, Arc<Vec<u8>>>>>,
}

impl FileMap {
    pub fn from_root_dir(root_dir: &str) -> Result<FileMap, Error> {
        let file = fs::File::open(root_dir)?;
        let metadata = file.metadata()?;

        if !metadata.is_dir() {
            return Err(Error::new(
                ErrorKind::NotADirectory,
                format!("Error: Root path is not a directory ({})", root_dir),
            ));
        }

        let head = Arc::new(FileNode::build_from_path(root_dir)?);

        Ok(FileMap {
            FULL_ROOT_PATH: root_dir.to_string(),
            head,
            lru: Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(20).unwrap()))),
        })
    }


    /// Returns a reference to the file node in the map for the given path
    /// Returns an `Arc<FileNode>` if the file is found, otherwise returns an `io::Error`
    /// Passing "" to this function will return a reference to the root node
    fn get_file_ref(&self, path: &str) -> Result<Arc<FileNode>, io::Error> {
        let path_split: Vec<&str> = path.split('/').collect();
        let mut current_node: Arc<FileNode> = self.head.clone();
        if path_split.len() > 1 {
            for i in 0..path_split.len() {
                if let Some(ref children) = current_node.children {
                    if let Some(z) = children.get(path_split[i]) {
                        current_node = z.clone();
                    } else {
                        return Err(io::Error::new(
                            ErrorKind::NotFound,
                            "File not found in file map",
                        ));
                    }
                } else {
                    return Err(io::Error::new(
                        ErrorKind::NotADirectory,
                        format!(
                            "Error, file {} is not a directory, cannot access",
                            path_split[i]
                        ),
                    ));
                }
            }
        }
        return Ok(current_node);
    }

    /// Confirms that a file is in the map, and then reads it from disk
    /// and returns it as an `Arc<Vec<u8>>`
    /// Returns `None` if the file is not found in the map or if there is an error reading it
    /// from disk.
    /// Remember when using not to add the 'root' directory to the path
    /// (e.g. if the root directory is "test_dir", use "testfile1.txt" as the path)
    async fn find_file_in_map(&self, path: &str) -> Result<Arc<Vec<u8>>, io::Error> {
        // Start opening file, allows for efficient tasking.
        let file = tokio::fs::File::open(format!("{}/{}", self.FULL_ROOT_PATH, path));
        
        // Check if the file exists in the map
        let r = self.get_file_ref(path)?;

        let mut buf: Vec<u8> = Vec::with_capacity(r.size as usize);

        let mut finished_file = file.await?;

        finished_file.read_to_end(&mut buf).await?;

        return Ok(Arc::new(buf));
    }

    /// Returns a file from the map if it exists, otherwise reads it from disk
    /// and caches it in the LRU cache for future access.
    /// Returns `None` if the file is not found in the map or if there is an error reading it
    /// from disk.
    pub async fn get_file(&self, path: &str) -> Result<Arc<Vec<u8>>, io::Error> {
        let mut lru = self.lru.lock().unwrap();

        let check_lru = lru.get(path);

        match check_lru {
            Some(s) => return Ok(s.clone()),
            None => {
                match self.find_file_in_map(path).await {
                    Ok(l) => {
                        lru.put(path.to_string(), l.clone());
                        return Ok(l);
                    }
                    Err(e) => {
                        return Err(e);
                    }
                };
            }
        }
    }

    
    
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DIR_PATH: &str = "../test_dir";
    #[test]
    fn test_working_dir() {
        let file_map = FileMap::from_root_dir(TEST_DIR_PATH).unwrap();
        assert_eq!(file_map.head.name, "test_dir");
        assert_eq!(file_map.head.size, 0);
        assert!(file_map.head.children.is_some());
        let children = file_map.head.children.as_ref().unwrap();
        assert_eq!(children.len(), 3); // test_dir has 3 children
        println!("Children: {:?}", children.keys().collect::<Vec<&String>>());
        assert!(children.contains_key("testfile1.txt"));
        assert!(children.contains_key("testfile2.mp4"));
        assert!(children.contains_key("test2"));
        assert_eq!(file_map.get_file_ref("").unwrap().name, file_map.head.name);
    }

    #[tokio::test]
    async fn test_file_reading() {
        let file_map = FileMap::from_root_dir(TEST_DIR_PATH).unwrap();
        let file = file_map.get_file("testfile1.txt").await.unwrap();
        assert_eq!(file.len(), 13); // test_file.txt has 13 bytes
    }
}
