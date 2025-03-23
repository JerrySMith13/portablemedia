use std::{collections::HashMap, fs::{self, read_dir}, 
    io::{Error, ErrorKind, Read}, 
    num::NonZeroUsize, 
    os::unix::fs::MetadataExt,
    sync::{Arc, Mutex}
};
type DirMap = HashMap<String, Arc<FileNode>>;
use lru::LruCache;

use crate::log::{self, log_err};

pub struct FileNode{
    pub name: String,
    pub size: u64,
    pub children: Option<DirMap>,
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
        let children: Option<DirMap>;
        

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
            let mut children_map: DirMap = HashMap::with_capacity(directory.size_hint().0);
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
    head: Arc<FileNode>,
    lru: Arc<Mutex<LruCache<String, Arc<Vec<u8>>>>>
}

impl FileMap {
    pub fn from_root_dir(root_dir: &str) -> Result<FileMap, Error> {
        let file = fs::File::open(root_dir)?;
        let metadata = file.metadata()?;

        if !metadata.is_dir(){
            return Err(Error::new(ErrorKind::NotADirectory, format!("Error: Root path is not a directory ({})", root_dir)));        
        }

        let head = Arc::new(FileNode::build_from_path(root_dir)?);

        Ok(FileMap{
            head,
            lru: Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(20).unwrap())))
        })


    }

    fn find_file_in_map(&self, path: &str) -> Option<Arc<Vec<u8>>>{
        let path_split: Vec<&str> = path.split('/').collect();
        let mut current_node: Arc<FileNode> = self.head.clone();
        for i in 0..path_split.len(){
            if let Some(ref children) = current_node.children{
                if let Some(z) = children.get(path_split[i]){
                    current_node = z.clone();
                }
                else{
                    return None;
                }
            }
            else { return None; }
        }
        
        let mut buf: Vec<u8> = Vec::with_capacity(current_node.size as usize);

        let mut file = match fs::File::open(path){
            Ok(k) => k,
            Err(_) => return None,
        };
        file.read_to_end(&mut buf);

        return Some(Arc::new(buf));



    }

    pub async fn get_file(&self, path:&str) -> Option<Arc<Vec<u8>>>{

        let mut lru = self.lru.lock().unwrap();

        let check_lru = lru.get(path);

        match check_lru{
            Some(s) => return Some(s.clone()),
            None => {
                match self.find_file_in_map(path){
                    Some(l) => {
                        lru.put(path.to_string(), l.clone());
                        return Some(l);
                    }
                    None => {
                        return None;
                    }
                };
            }
        }
        
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn test_working_dir(){
        let dir_path = "../../test_dir";
        let file_map = FileMap::from_root_dir(dir_path).unwrap();
        

    }
}