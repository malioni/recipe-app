/// storage module meant to handle interactions with the database.
use std::{
    fs::File,
    io::{self, Write, Read},
    path::PathBuf,
};
/// Writes a string to a file.
///
/// # Arguments
///
/// * `PathBuf` - The path to the file where the content will be written.
/// * `content` - The string content to be written to the file.
///
/// # Returns
/// 
/// Returns `Ok(())` if the content was successfully written to the file.
/// Returns `Err(io::Error)` if there was an error during the file operations.
///
/// # Errors
///
/// This function will return an error if:
/// * The file cannot be created or opened for writing.
/// * There is an error writing the content to the file.
///
pub fn write_to_file(filename: &PathBuf, content: &str) -> io::Result<()> {
    // Create or open the file for writing
    let mut file = match File::create(filename) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Error creating file {}: {}", filename.display(), e);
            return Err(e);
        }
    };

    // Write the content to the file
    match file.write_all(content.as_bytes()) {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("Error writing to file {}: {}", filename.display(), e);
            Err(e)
        }
    }
}

/// Reads the content of a file into a string.
///
/// # Arguments
///
/// * `PathBuf` - The path to the file to be read.
///
/// # Returns
/// 
/// Returns `Ok(String)` containing the content of the file if successful.
/// Returns `Err(io::Error)` if there was an error during the file operations.
///
/// # Errors
///
/// This function will return an error if:
/// * The file cannot be opened for reading.
/// * There is an error reading the content of the file.
///
pub fn read_from_file(filename: &PathBuf) -> io::Result<String> {
    // Open the file for reading
    let mut file = match File::open(filename) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Error opening file for reading {}: {}", filename.display(), e);
            return Err(e);
        }
    };

    // Read the content of the file into a string
    let mut content = String::new();
    match file.read_to_string(&mut content) {
        Ok(_) => Ok(content),
        Err(e) => {
            eprintln!("Error reading from file {}: {}", filename.display(), e);
            Err(e)
        }
    }
}
