/// network library meant to handle everything related to the connection, including
/// the connection itself and the response to endpoints
use std::{
    io::{self, prelude::*, BufReader},
    net::TcpStream,
    fs,
    path::Path,
};

/// handle the connection received, read the first line and return appropriate response
///
/// # Arguments
///
/// * `stream` - object representing incoming connection to the server
///
/// # Returns
/// 
/// Returns `Ok()` if the stream request was identified, recognized and responded to
/// Returns `Err(io:Error)` if there's an issue with the request or the response
///
/// # Errors
///
/// This function will return an error if:
/// - The first line of the incoming request cannot be read
/// - The html file needed for response cannot be found
/// - There's an issue with sending the response
///
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

    let response = match &request_line[..] {
        "GET / HTTP/1.1" => match html_file_response("HTTP/1.1 200 OK", "recipe-page.html") {
            Ok(response) => response,
            Err(e) => {
                eprintln!("Error while requesting html file response: {e:?}");
                return Err(e);
            }
        },
        _ => match html_file_response("HTTP/1.1 404 NOT FOUND", "404.html") {
            Ok(response) => response,
            Err(e) => {
                eprintln!("Error while requesting html file response: {e:?}");
                return Err(e);
            }
        },
        };

    if let Err(e) = stream.write_all(response.as_bytes()) {
        eprintln!("Error while sending the response: {e:?}");
        return Err(e);
    };

    Ok(())
}

fn html_file_response(status_line: &str, filename: &str) -> io::Result<String> {
    let directory = "html";
    let path = Path::new(directory).join(filename);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(e) => {
            eprintln!("Error while opening an html file at location {}: {}", path.display(), e);
            return Err(e);
        }};

    let length = contents.len();
    Ok(format!("{status_line}\r\nContent-Length: {length}\r\n\r\n{contents}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        thread,
        net::{TcpListener, TcpStream},
    };

    #[test]
    fn test_handle_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn server thread
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle_connection(stream);
            }
        });

        // Connect as client
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
    }
}
