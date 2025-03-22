use std::{collections::HashMap, fs::{self, read_dir}, 
    io::{Error, ErrorKind}, 
    num::NonZeroUsize, 
    os::unix::fs::MetadataExt,
    sync::Arc
};

use lru::LruCache;

use crate::log::{self, log_err};

pub struct FileNode{
    pub name: String,
    pub size: u64,
    pub children: Option<HashMap<String, Arc<FileNode>>>,
}

impl FileNode{
    fn build_from_path(path: &str) -> Result<FileNode, Error>{
        
        let file = fs::File::open(path)?;
        let metadata = file.metadata()?;
        
        let mut size: u64 = 0;
        let name = match path.split("/").last(){
            Some(s) => s.to_string(),
            None => return Err(Error::new(ErrorKind::Other, format!("Error in trying to assign name to file {}", path)))
        };
        let children: Option<HashMap<String, Arc<FileNode>>>;
        

        if metadata.is_symlink(){
            return Err(Error::new(
                ErrorKind::Other, 
                format!("Error: file {} is a symlink (symlinks are not currently supported)", path)
            ));
        }
        if metadata.is_file(){
            size = metadata.size();
            children = None;
        }
        else{
            //Safe unwrap because we know for a fact it's a directory, nothing about the file state can change
            let directory = read_dir(path).unwrap();
            let mut children_map: HashMap<String, Arc<FileNode>> = HashMap::with_capacity(directory.size_hint().0);
            for file in directory{
                if file.is_err(){
                    log_err(
                        format!("Error in reading a file in directory {}, skipping file", path).as_str(), 
                        log::LogPriority::Middle);
                    continue;
                }
                let file_name = file.unwrap().file_name().into_string();
                if file_name.is_err(){
                    log_err(
                        format!("Error: filename {} in directory {} not valid unicode, skipping file", file_name.unwrap_err().to_string_lossy().into_owned(), path).as_str(), 
                        log::LogPriority::Middle);
                    continue;
                }
                let file_name = file_name.unwrap();
                children_map.insert(file_name.clone(), Arc::new(FileNode::build_from_path(format!("{}/{}", path, file_name).as_str())?));
            }
            children = Some(children_map);

        }
        return Ok(FileNode{
            name,
            size,
            children
        })
        
    }
}

pub struct FileMap {
    head: FileNode,
    lru: LruCache<String, Vec<u8>>
}

impl FileMap {
    pub fn from_root_dir(root_dir: &str) -> Result<FileMap, Error> {
        let file = fs::File::open(root_dir)?;
        let metadata = file.metadata()?;

        if !metadata.is_dir(){
            return Err(Error::new(ErrorKind::NotADirectory, format!("Error: Root path is not a directory ({})", root_dir)));        
        }

        let head = FileNode::build_from_path(root_dir)?;

        Ok(FileMap{
            head,
            lru: LruCache::new(NonZeroUsize::new(20).unwrap())
        })


    }
}

#[cfg(test)]
mod tests{
    
}