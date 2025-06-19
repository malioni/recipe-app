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

pub fn write_to_file(filename: &str, content: &str) -> io::Result<()> {
    // Create or open the file for writing
    let mut file = match File::create(filename) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Error creating file {}: {}", filename, e);
            return Err(e);
        }
    };

    // Write the content to the file
    match file.write_all(content.as_bytes()) {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("Error writing to file {}: {}", filename, e);
            Err(e)
        }
    }
}

pub fn read_from_file(filename: &str) -> io::Result<String> {
    // Open the file for reading
    let mut file = match File::open(filename) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Error opening file for reading {}: {}", filename, e);
            return Err(e);
        }
    };

    // Read the content of the file into a string
    let mut content = String::new();
    match file.read_to_string(&mut content) {
        Ok(_) => Ok(content),
        Err(e) => {
            eprintln!("Error reading from file {}: {}", filename, e);
            Err(e)
        }
    }
}
