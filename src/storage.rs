/// storage module meant to handle interactions with the database.
use std::{
    fs::File,
    io::{self, Write, Read},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct Recipe {
    name: String,
    picture: String,
    ingredients: Vec<String>,
    instructions: Vec<String>,
}

fn write_to_file(filename: &str, content: &str) -> io::Result<()> {
    let mut file = File::create(filename)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

fn read_from_file(filename: &str) -> io::Result<String> {
    let mut file = File::open(filename)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}
