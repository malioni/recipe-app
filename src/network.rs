use std::{
    io::{self, prelude::*, BufReader},
    net::TcpStream,
    fs,
    path::Path,
};

pub fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    let buf_reader = BufReader::new(&stream);
    let mut lines = buf_reader.lines();
    let request_line = match lines.next() {
        Some(Ok(line)) => line,
        Some(Err(e)) => {
            eprintln!("Error reading first line: {e:?}");
            return Err(e);
        }
        None => {
            eprintln!("No data received from client.");
            return Ok(());
        }};

    let (status_line, filename) = match &request_line[..] {
        "GET / HTTP/1.1" => ("HTTP/1.1 200 OK", "recipe-page.html"),
        _ => ("HTTP/1.1 404 NOT FOUND", "404.html"),
        };

    let directory = "html";
    let path = Path::new(directory).join(filename);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(e) => {
            eprintln!("Error while opening an html file at location {}: {}", path.display(), e);
            return Err(e);
        }};
    let length = contents.len();

    let response = format!("{status_line}\r\nContent-Length: {length}\r\n\r\n{contents}");

    if let Err(e) = stream.write_all(response.as_bytes()) {
        eprintln!("Error while sending the response: {e:?}");
        return Err(e);
    };

    Ok(())
}




        
