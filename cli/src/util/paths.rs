use std::{env, io, path::PathBuf};

pub fn get_nearby_bin(file: &str) -> io::Result<PathBuf> {
    let curent_exe = env::current_exe()?;

    let parent = curent_exe.parent().unwrap();

    let ret = parent.join(&file);

    Ok(ret)
}
